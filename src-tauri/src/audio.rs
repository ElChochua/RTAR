use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleRate, Stream, StreamConfig};
use ringbuf::{Consumer, Producer, HeapRb};
use std::error::Error;
use std::sync::Arc;
use tokio::net::UdpSocket;

/// Initializes the audio system, looks for the default output device
/// and configures the stream with RTABC specifications:
/// - 2 Channels (Stereo)
/// - 48,000 Hz
/// - f32 (Floating Point 32-bits)
pub fn setup_audio_stream() -> Result<Stream, Box<dyn Error>> {
    // 1. We get the audio "Host" (Alsa/PulseAudio on Linux, CoreAudio on Mac, WASAPI on Windows, AAudio on Android)
    let host = cpal::default_host();

    // 2. We look for the default output audio device (the speakers of the phone/PC)
    let device = host
        .default_output_device()
        .ok_or("Default output audio device not found")?;

    println!(
        "Selected output audio device: {}",
        device.name()? // Print the name of the selected output audio device
    );

    // 3. We force our audio configuration chosen in RTABC
    let config = StreamConfig {
        channels: 2,                            // Stereo
        sample_rate: SampleRate(48000),         // 48kHz
        
        // Support for Wireless (Bluetooth)! Instead of demanding 256 fixed samples
        // to the sound card (which crashes BT headphones in silence),
        // we let the operating system and the A2DP driver choose their ideal buffer.
        buffer_size: cpal::BufferSize::Default, 
    };

    // 4. We build the concurrent queue (Ring Buffer) to pass the float32 from the network to the audio
    // Extreme Noise Resistance Adjustment (Bluetooth):
    // The Wi-Fi and BT chips share the same physical antenna in phones and the 2.4Ghz frequency.
    // Turning on BT can cause Wi-Fi to drop packets.
    let ring_buffer = HeapRb::<f32>::new(32768);
    let (producer, mut consumer) = ring_buffer.split();

    // 5. We create the stream (it doesn't start playing yet, it's just built)
    println!("Configuring Output Stream: 2 Channels, 48000Hz, f32 (Low Latency)");

    // Internal state for Soft-Start "Pre-Buffering". 
    // Will start as 'false' mutating to 'true' once the network gathers enough data.
    let mut is_playing = false;
    let prebuffer_threshold = 480; // Ultra-Low Latency: Same as 5ms of audio (48,000Hz Stereo = 96k/sec)

    let err_fn = |err| eprintln!("An error occurred in the CPAL Audio thread: {}", err);

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let available = consumer.len();
            
            // 1. Pre-Buffering Logic. We don't play anything if the queue is almost dead
            if !is_playing {
                if available < prebuffer_threshold {
                    // Keep injecting mathematical silence without Tsss
                    for sample in data.iter_mut() { *sample = 0.0; }
                    return; // Exit immediately from the callback
                }
                // The threshold has been crossed! Release the dam
                is_playing = true;
            } else if available == 0 {
                // Buffer empty (Wi-Fi died temporarily). Freeze the playback.
                is_playing = false;
                for sample in data.iter_mut() { *sample = 0.0; }
                return;
            }

            // 2. Regular extraction if pre-buffer is exceeded
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

/// Listens passively on UDP port 5001 for all audio sent by the PC.
/// 
/// **Architectural Decision:** We transform bytes to Floats with zero-copy casting (`bytemuck`).
async fn receive_audio_udp(
    mut producer: Producer<f32, Arc<HeapRb<f32>>>
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

    println!("Audio Antenna ready. Listening for converted G.711 µ-Law packets on UDP port 5000 (SO_REUSEADDR Active) with Auto-Punch...");

    let mut buf = [0; 2048]; // Buffer for what arrives via UDP

    // Pure inverse auxiliary function in Rust: µ-Law (u8) to floating PCM (f32)
    // Scale from logarithmic value to linear range
    #[inline]
    fn ulaw_to_f32(ulaw: u8) -> f32 {
        let ulaw = !ulaw;
        let sign = ulaw & 0x80;
        let exponent = (ulaw >> 4) & 0x07;
        let mantissa = ulaw & 0x0F;
        let mut sample = (((mantissa as i32) << 3) + 132) << exponent;
        sample -= 132;
        let pcm = if sign != 0 { -sample } else { sample };
        (pcm as f32) / 32767.0
    }

    loop {
        let (size, _) = socket.recv_from(&mut buf).await?;
        let packet_data = &buf[..size];

        // Anti-Buffer-Bloat ("Anti-Ping" infinito)
        // 48,000Hz estéreo = 96,000 muestras por segundo (f32). 
        // If there are more than 4,800 samples (equivalent to 50ms), it means that 
        // UDP bursts arrived (Network Jitter due to Bluetooth).
        // The solution? We discard a piece of the old buffer (jumping in time)
        // to recover the "real time" instead of suffering 1 second of latency.
        if producer.len() > 9600 { // If we accumulate > 100ms of theoretical delay
            // Pop out old data from the ring buffer silently (we ignore it)
            // In a Ring Buffer SPSC we cannot empty the "producer" directly,
            // but we can skip. Unfortunately rtrb/HeapRb requires doing it from the side
            // of the consumer or cleaning. Since producer does not have `clear()`, 
            // simply let cpal finish draining it or avoid inserting more
            // until it goes down, BUT avoiding inserting more creates hard cuts (we cut the new one).
            // The ideal is not to insert if it's full! The old packets are already inside.
            // So we will simply ignore new packets temporarily if the buffer is critically full.
            // (Let CPAL consume the queue).
        }

        // Instant expansion: we iterate byte by byte (µ-Law) y return f32
        // Only inject if the buffer is not critically stuck
        if producer.len() < 19200 { // Max ceiling 200ms
            for &byte in packet_data {
                let sample = ulaw_to_f32(byte);
                let _ = producer.push(sample);
            }
        }
    }
}
