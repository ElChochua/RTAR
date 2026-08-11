use crate::audio_codec::{self, AudioDecoder};
use crate::jitter::{AudioJitterBuffer, BufferedAudioPacket, PlayoutItem};
use crate::protocol::{self, Codec, MediaKind, FLAG_DISCONTINUITY};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleRate, Stream, StreamConfig};
use ringbuf::{HeapRb, Producer};
use std::error::Error;
use std::sync::Arc;
use tokio::net::UdpSocket;

/// Initializes the audio system, looks for the default output device
/// and configures the stream with RTABC specifications:
/// - 2 Channels (Stereo)
/// - 48,000 Hz
/// - f32 (Floating Point 32-bits)
pub fn setup_audio_stream() -> Result<Stream, Box<dyn Error>> {
    // We get the audio "Host" (Alsa/PulseAudio on Linux, CoreAudio on Mac, WASAPI on Windows, AAudio on Android)
    let host = cpal::default_host();

    // We look for the default output audio device (the speakers of the phone/PC)
    let device = host
        .default_output_device()
        .ok_or("Default output audio device not found")?;

    println!(
        "Selected output audio device: {}",
        device.name()? // Print the name of the selected output audio device
    );

    // We force our audio configuration chosen in RTABC
    let config = StreamConfig {
        channels: 2,                    // Stereo
        sample_rate: SampleRate(48000), // 48kHz

        // Support for Wireless (Bluetooth)! Instead of demanding 256 fixed samples
        // to the sound card (which crashes BT headphones in silence),
        // we let the operating system and the A2DP driver choose their ideal buffer.
        buffer_size: cpal::BufferSize::Default,
    };

    // 4. We build the concurrent queue (Ring Buffer) to pass the float32 from the network to the audio
    // Extreme Noise Resistance Adjustment (Bluetooth):
    // The Wi-Fi and BT chips share the same physical antenna in phones and the 2.4Ghz frequency.
    // Turning on BT can cause Wi-Fi to drop packets.
    let ring_buffer = HeapRb::<f32>::new(audio_codec::FRAME_SAMPLES * 5);
    let (producer, mut consumer) = ring_buffer.split();

    // We create the stream (it doesn't start playing yet, it's just built)
    println!("Configuring Output Stream: 2 Channels, 48000Hz, f32 (Low Latency)");

    // Internal state for Soft-Start "Pre-Buffering".
    // Will start as 'false' mutating to 'true' once the network gathers enough data.
    let mut is_playing = false;
    let prebuffer_threshold = audio_codec::FRAME_SAMPLES;

    let err_fn = |err| eprintln!("An error occurred in the CPAL Audio thread: {}", err);

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let available = consumer.len();

            // Pre-Buffering Logic. We don't play anything if the queue is almost dead
            if !is_playing {
                if available < prebuffer_threshold {
                    // Keep injecting mathematical silence without Tsss
                    for sample in data.iter_mut() {
                        *sample = 0.0;
                    }
                    return; // Exit immediately from the callback
                }
                // The threshold has been crossed! Release the dam
                is_playing = true;
            } else if available == 0 {
                // Buffer empty (Wi-Fi died temporarily). Freeze the playback.
                is_playing = false;
                for sample in data.iter_mut() {
                    *sample = 0.0;
                }
                return;
            }

            // Regular extraction if pre-buffer is exceeded
            let safe_read_count = std::cmp::min(available, data.len());
            let even_read_count = safe_read_count - (safe_read_count % 2); // Truncate to even

            let read_count = consumer.pop_slice(&mut data[..even_read_count]);

            // If we are missing data to read (slight underflow on the local side)
            // Fill the rest with silence to avoid infinite noise (RAM Ghosting)
            for sample in data[read_count..].iter_mut() {
                *sample = 0.0;
            }
        },
        err_fn,
        None, // Timeout opcional
    )?;

    // Play the stream (we need to keep the `stream` variable alive or it will stop)
    stream.play()?;
    println!("¡Stream de audio local inicializado y consumiendo memoria!");

    // 6. Start receiving audio UDP in the background
    tauri::async_runtime::spawn(async move {
        if let Err(e) = receive_audio_udp(producer).await {
            eprintln!("Error receiving audio: {}", e);
        }
    });

    Ok(stream)
}

/// Receives versioned Opus packets, reorders them and plays on a fixed 10 ms clock.
async fn receive_audio_udp(
    mut producer: Producer<f32, Arc<HeapRb<f32>>>,
) -> Result<(), Box<dyn Error>> {
    let socket2 = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;

    // Reuse the port even if the "Ghost" of the previous app still exists
    socket2.set_reuse_address(true)?;
    socket2.set_nonblocking(true)?;

    let addr: std::net::SocketAddr = "0.0.0.0:5000".parse().unwrap();
    socket2.bind(&addr.into())?;

    let std_socket: std::net::UdpSocket = socket2.into();
    let socket = UdpSocket::from_std(std_socket)?;

    // Since the Discovery Ping now travels through a random port, the Android firewall
    // will close port 5000 blocking the audio input from the PC server.
    // To avoid this, we fire a "salvo shot" from the newly opened port 5000
    // directly to port 8888 of the PC. This pierces the Gateway NAT.
    let punch_msg = b"RTABC_AUDIO_HOLE_PUNCH";
    let pc_target = "255.255.255.255:8888"; // We use broadcast to avoid having to inject the IP here
    socket.set_broadcast(true)?;
    if let Err(e) = socket.send_to(punch_msg, pc_target).await {
        eprintln!("Notice: Failed to punch port 5000 (NAT Hole Punch): {}", e);
    }
    // Return the socket to its normal state
    socket.set_broadcast(false)?;

    println!("Audio receiver ready. Listening for Opus packets on UDP port 5000...");

    let mut buf = [0; protocol::MAX_DATAGRAM_LEN];
    let mut jitter = AudioJitterBuffer::new();
    let mut decoder = AudioDecoder::new();
    let mut active_stream_id = None;
    let mut playout = tokio::time::interval(std::time::Duration::from_millis(
        audio_codec::FRAME_DURATION_MS as u64,
    ));
    playout.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            received = socket.recv_from(&mut buf) => {
                let (size, _) = received?;
                let (header, packet_data) = match protocol::decode_datagram(&buf[..size]) {
                    Ok(packet) => packet,
                    Err(_) => continue,
                };
                if header.media_kind != MediaKind::Audio || header.codec != Codec::Opus {
                    continue;
                }

                let stream_changed = active_stream_id != Some(header.stream_id);
                if stream_changed || header.flags & FLAG_DISCONTINUITY != 0 {
                    jitter.reset();
                    decoder.reset();
                    active_stream_id = Some(header.stream_id);
                }

                jitter.insert(BufferedAudioPacket {
                    sequence: header.sequence,
                    timestamp_us: header.timestamp_us,
                    flags: header.flags,
                    payload: packet_data.to_vec(),
                });
            }

            _ = playout.tick() => {
                let decoded = match jitter.pop() {
                    PlayoutItem::Waiting => None,
                    PlayoutItem::Packet(packet) => decoder.decode(&packet.payload).ok(),
                    PlayoutItem::Missing { next_payload, .. } => {
                        match next_payload {
                            Some(packet) => decoder.recover_fec(&packet).ok(),
                            None => Some(decoder.conceal()),
                        }
                        .or_else(|| Some(decoder.conceal()))
                    }
                    PlayoutItem::Reset => {
                        decoder.reset();
                        None
                    }
                };

                if let Some(pcm) = decoded {
                    if pcm.len() == audio_codec::FRAME_SAMPLES
                        && producer.free_len() >= pcm.len()
                    {
                        let _ = producer.push_slice(&pcm);
                    }
                }
            }
        }
    }
}
