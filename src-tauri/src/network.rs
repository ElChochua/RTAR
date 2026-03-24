use std::error::Error;
use tokio::net::UdpSocket;
use tauri::{AppHandle, Emitter};

/// Listens and transmits UDP packets to find the main server using a Ping/Pong model.
///
/// **Architectural Decision**: Instead of waiting passively, our client must take the initiative.
/// We send a "Ping" to all devices on the local network (Broadcast) to port 8888.
/// If the RTABC server is alive, it will respond with a "Pong".
pub async fn listen_for_server(app: AppHandle) -> Result<(), Box<dyn Error>> {
    // 1. We bind the Scanner (Discovery) to an Ephemeral/Random Port (0.0.0.0:0).
    // This way, we leave the coveted port 5000 FREE for *only* the 
    // Audio module (audio.rs) to occupy it without Linux/Android balancing the
    // packets destroying the music.
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    
    // 2. Allow the socket to send broadcast messages (to the entire local subnet)
    socket.set_broadcast(true)?;
    
    println!("Starting UDP Discovery (Ping) to port 8888...");

    let mut buf = [0; 1024];

    loop {
        // Send the heartbeat to the entire local subnet (Broadcast) on the PC port
        let ping_msg = b"RTABC_DISCOVERY_PING";
        if let Err(e) = socket.send_to(ping_msg, "255.255.255.255:8888").await {
            eprintln!("Error sending ping of discovery: {}", e);
        }

        // 4. Wait for "Pong". Timeout fast because if the PC is alive, it answers almost instantly.
        match tokio::time::timeout(std::time::Duration::from_millis(500), socket.recv_from(&mut buf)).await {
            Ok(Ok((size, addr))) => {
                let msg = String::from_utf8_lossy(&buf[..size]);
                
                // 5. The server PC answered us
                if msg == "RTABC_DISCOVERY_PONG" {
                    let server_ip = addr.ip().to_string();
                    
                    // Emit an event every second to ensure that the UI receives the event
                    // even if the UI is reloaded.
                    if let Err(e) = app.emit("server_found", &server_ip) {
                        eprintln!("Error al emitir evento al frontend: {}", e);
                    }
                }
            },
            Ok(Err(e)) => {
                eprintln!("Error receiving pong: {}", e);
            },
            Err(_) => {
                // Timeout. Nobody answered, PC busy or not visible temporarily.
            }
        }
        
        // We sleep 1 heartbeat per second. This way, when you click on 'Connect Audio',
        // the PC will continue to receive this heartbeat on the parallel ephemeral port.
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
    }
}
