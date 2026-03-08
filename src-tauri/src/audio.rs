use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleRate, Stream, StreamConfig};
use ringbuf::{Consumer, Producer, HeapRb};
use std::error::Error;
use std::sync::Arc;
use tokio::net::UdpSocket;

/// Inicializa el sistema de audio, busca el dispositivo de salida predeterminado
/// y configura el stream con las especificaciones de RTABC:
/// - 2 Canales (Estéreo)
/// - 48,000 Hz
/// - f32 (Floating Point 32-bits)
pub fn setup_audio_stream() -> Result<Stream, Box<dyn Error>> {
    // 1. Obtenemos el "Host" de audio (Alsa/PulseAudio en Linux, CoreAudio en Mac, WASAPI en Windows, AAudio en Android)
    let host = cpal::default_host();

    // 2. Buscamos el dispositivo de salida por defecto (los parlantes del celular/PC)
    let device = host
        .default_output_device()
        .ok_or("No se encontró un dispositivo de salida de audio predeterminado")?;

    println!(
        "Dispositivo de salida de audio seleccionado: {}",
        device.name()?
    );

    // 3. Forzamos nuestra configuración de audio elegida en RTABC
    let config = StreamConfig {
        channels: 2,                            // Estéreo
        sample_rate: SampleRate(48000),         // 48kHz
        
        // ¡Soporte para Inalámbricos (Bluetooth)! En lugar de exigirle 256 muestras
        // fijas a la tarjeta de sonido (lo cual crashea los audífonos BT en silencio),
        // dejamos que el sistema operativo y el driver A2DP elijan su buffer ideal.
        buffer_size: cpal::BufferSize::Default, 
    };

    // 4. Construimos la cola concurrente (Ring Buffer) para pasar los float32 desde la red al audio
    // Ajuste de Ruido Extremo (Resistencia a Bluetooth):
    // El chip de Wi-Fi y BT comparten la misma antena física en los teléfonos y frecuencia de 2.4Ghz.
    // Al prender el BT, el Wi-Fi se entrecorta perdiendo paquetes UDP.
    // Usar 32768 muestras nos da aprox ~340ms de salvavidas acústico para que aunque la
    // antena BT trabe el Wi-Fi o busque redes, el audio siga sonando sin estática ni cortes feos.
    let ring_buffer = HeapRb::<f32>::new(32768);
    let (producer, mut consumer) = ring_buffer.split();

    // 5. Creamos el stream (aún no empieza a reproducir, solo se construye)
    println!("Configurando Stream de Salida: 2 Canales, 48000Hz, f32 (Low Latency)");

    // Estado interno para el Soft-Start "Pre-Buffering". 
    // Empezará en 'false' mutando a 'true' una vez que la red junte suficientes datos.
    let mut is_playing = false;
    let prebuffer_threshold = 480; // Ultra-Low Latency: Mismo que 5ms de audio (48,000Hz Estéreo = 96k/seg)

    let err_fn = |err| eprintln!("Un error ocurrió en el hilo de Audio de CPAL: {}", err);

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let available = consumer.len();
            
            // 1. Lógica de Pre-Buffering. No reproducimos nada si la cola está casi muerta
            if !is_playing {
                if available < prebuffer_threshold {
                    // Seguimos inyectando silencio matemático sin Tsss
                    for sample in data.iter_mut() { *sample = 0.0; }
                    return; // Salimos de inmediato del callback
                }
                // ¡Se cruzó el umbral! Desatamos la presa
                is_playing = true;
            } else if available == 0 {
                // Buffer vacío (Wi-Fi murió temporalmente). Congelamos la reproducción.
                is_playing = false;
                for sample in data.iter_mut() { *sample = 0.0; }
                return;
            }

            // 2. Extracción regular si pre-buffer superado
            let safe_read_count = std::cmp::min(available, data.len());
            let even_read_count = safe_read_count - (safe_read_count % 2); // Truncar a par
            
            let read_count = consumer.pop_slice(&mut data[..even_read_count]);
            
            // Si nos faltaron datos por leer (underflow leve del lado local)
            // Llenamos el resto con silencio para evitar ruido infinito (Ghosting de RAM)
            for sample in data[read_count..].iter_mut() {
                *sample = 0.0;
            }
        },
        err_fn,
        None, // Timeout opcional
    )?;

    // Reproducimos (necesitamos mantener la variable `stream` viva o dejará de sonar)
    stream.play()?;
    println!("¡Stream de audio local inicializado y consumiendo memoria!");

    // 6. Arrancamos de fondo la recepción del audio UDP, pasándole el extremo "Productor" del anillo
    tauri::async_runtime::spawn(async move {
        if let Err(e) = receive_audio_udp(producer).await {
            eprintln!("Error en la recepción de audio UDP: {}", e);
        }
    });

    Ok(stream)
}

/// Escucha pasivamente en el puerto UDP 5001 todo el audio que envía la PC.
/// 
/// **Decisión Arquitectónica:** Transformamos bytes a Floats con zero-copy casting (`bytemuck`).
async fn receive_audio_udp(
    mut producer: Producer<f32, Arc<HeapRb<f32>>>
) -> Result<(), Box<dyn Error>> {
    let socket2 = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;

    // Magia Negra TCP/UDP: ¡Robarse el puerto incluso si el "Fantasma" de la app anterior aún existe!
    socket2.set_reuse_address(true)?;
    socket2.set_nonblocking(true)?;

    let addr: std::net::SocketAddr = "0.0.0.0:5000".parse().unwrap();
    socket2.bind(&addr.into())?;

    // Convertimos de vuelta al mundo asíncrono de Tokio
    let std_socket: std::net::UdpSocket = socket2.into();
    let socket = UdpSocket::from_std(std_socket)?;

    // --- MAGIA: NAT Auto-Punching (Perforación de Firewall) ---
    // Como ahora el Ping de Discovery viaja por un puerto aleatorio, el firewall 
    // de Android cerrará el puerto 5000 bloqueando la entrada de audio del servidor PC.
    // Para evitarlo, disparamos una "bala de salva" desde el puerto 5000 recién
    // abierto directo hacia el puerto 8888 de la PC. Esto perfora el Gateway NAT.
    let punch_msg = b"RTABC_AUDIO_HOLE_PUNCH";
    let pc_target = "255.255.255.255:8888"; // Usamos broadcast para no tener que inyectar la IP aquí
    socket.set_broadcast(true)?;
    if let Err(e) = socket.send_to(punch_msg, pc_target).await {
        eprintln!("Aviso: Fallo al perforar el puerto 5000 (NAT Hole Punch): {}", e);
    }
    // Devolvemos el socket a su estado normal
    socket.set_broadcast(false)?;

    println!("Antena de Audio lista. Escuchando paquetes convertidos G.711 µ-Law en el puerto UDP 5000 (SO_REUSEADDR Activo) con Auto-Punch...");

    let mut buf = [0; 2048]; // Buffer para lo que llega por UDP

    // Función auxiliar inversa pura en Rust: µ-Law (u8) a flotante PCM (f32)
    // Escala del valor logarítmico a rango lineal
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

        // Expansión instantánea: iteramos byte por byte (µ-Law) y devolvemos f32
        for &byte in packet_data {
            let sample = ulaw_to_f32(byte);
            let _ = producer.push(sample);
        }
    }
}
