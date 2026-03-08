use std::error::Error;
use tokio::net::UdpSocket;
use tauri::{AppHandle, Emitter};

/// Escucha y transmite paquetes UDP para encontrar el servidor principal usando un modelo de Ping/Pong.
///
/// **Decisión Arquitectónica**: En lugar de esperar pasivamente, nuestro cliente debe tomar la iniciativa.
/// Enviamos un "Ping" a todos los dispositivos en la red local (Broadcast) al puerto 8888.
/// Si el servidor RTABC está vivo, nos responderá con un "Pong".
pub async fn listen_for_server(app: AppHandle) -> Result<(), Box<dyn Error>> {
    // 1. Vinculamos el Escáner (Discovery) en un Puerto Efímero/Aleatorio (0.0.0.0:0).
    // De esta manera, dejamos el codiciado puerto 5000 LIBRE para que *solo* el 
    // módulo de Audio (audio.rs) lo ocupe sin que Linux/Android balancee los
    // paquetes destruyendo la música.
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    
    // 2. Permitimos que el socket envíe mensajes de broadcast (a toda la subred local)
    socket.set_broadcast(true)?;
    
    println!("Iniciando Discovery UDP (Ping) hacia el puerto 8888...");

    let mut buf = [0; 1024];

    loop {
        // Enviar el latido a toda la subred local (Broadcast) en el puerto de la PC
        let ping_msg = b"RTABC_DISCOVERY_PING";
        if let Err(e) = socket.send_to(ping_msg, "255.255.255.255:8888").await {
            eprintln!("Error enviando ping de descubrimiento: {}", e);
        }

        // 4. Esperar "Pong". Timeout rápido porque si la PC está viva, contesta casi instantáneamente.
        match tokio::time::timeout(std::time::Duration::from_millis(500), socket.recv_from(&mut buf)).await {
            Ok(Ok((size, addr))) => {
                let msg = String::from_utf8_lossy(&buf[..size]);
                
                // 5. El servidor PC nos contestó
                if msg == "RTABC_DISCOVERY_PONG" {
                    let server_ip = addr.ip().to_string();
                    
                    // Emitir el evento de forma CONTÍNUA (cada segundo) para asegurar que
                    // si Android se desconecta de sus auriculares y su UI aprieta "window.reload()",
                    // reciba la señal al instante en la nueva pantalla.
                    if let Err(e) = app.emit("server_found", &server_ip) {
                        eprintln!("Error al emitir evento al frontend: {}", e);
                    }
                }
            },
            Ok(Err(e)) => {
                eprintln!("Error recibiendo pong: {}", e);
            },
            Err(_) => {
                // Timeout. Nadie respondió, PC ocupada o no visible temporalmente.
            }
        }
        
        // Dormimos 1 latido por segundo. Así, cuando aprietes en 'Conectar Audio',
        // el PC seguirá recibiendo este latido por el puerto efímero paralelo.
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
}
