mod network;
mod audio;
use lazy_static::lazy_static;
use std::sync::Mutex;

// A raw pointer (*mut) does not implement Send/Sync by default (Rust's safety for threads).
// We make a manual container swearing to Rust that we control its access via Mutex.
struct StreamHandle(*mut cpal::Stream);
unsafe impl Send for StreamHandle {}
unsafe impl Sync for StreamHandle {}

lazy_static! {
    static ref GLOBAL_AUDIO_STREAM: Mutex<Option<StreamHandle>> = Mutex::new(None);
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn start_audio(ip: &str) -> Result<String, String> {
    println!("Frontend requested to connect to audio IP: {}", ip);
    
    // We try to initialize CPAL and save it
    match audio::setup_audio_stream() {
        Ok(stream) => {
            // We pack it in the Heap (Box), convert it to Raw Pointer and save it in the global
            let stream_ptr = Box::into_raw(Box::new(stream));
            let mut global_stream = GLOBAL_AUDIO_STREAM.lock().unwrap();
            
            // If for some reason there was a live one, we kill it (to avoid memory leaks)
            if let Some(old_handle) = global_stream.take() {
                unsafe {
                    drop(Box::from_raw(old_handle.0));
                }
            }

            *global_stream = Some(StreamHandle(stream_ptr));
            Ok(format!("Local audio stream initialized for {}", ip))
        },
        Err(e) => Err(format!("Failed to initialize CPAL: {}", e))
    }
}

/// Destroys the crashed thread ("The requested device is no longer available")
/// Cleans up memory and cuts the current Ring Buffer so it can be restarted
#[tauri::command]
fn restart_audio() -> Result<String, String> {
    println!("Frontend requested to restart the Audio subsystem (Hot-Swap)...");
    
    let mut global_stream = GLOBAL_AUDIO_STREAM.lock().unwrap();
    if let Some(handle) = global_stream.take() {
        unsafe {
            // By reconstructing the Box and letting it fall out of scope, CPAL executes
            // the internal drop(), releasing the 'dead IAudioTrack' of Android.
            drop(Box::from_raw(handle.0));
        }
    }
    Ok("Audio service stopped successfully.".into())
}

/// Envía comandos multimedia (Play, Pause, Next) hacia el servidor (PC).
/// 
/// **Arquitectura**: Usamos un UdpSocket síncrono estándar de la stdlib porque esto 
/// es un disparo "fire and forget" muy rápido. No vale la pena bloquear tareas asíncronas
/// para enviar unos bytes que representan un comando de control y cerrarse.
#[tauri::command]
fn send_media_command(ip: &str, command: &str) -> Result<String, String> {
    println!("[Comando Multimedia] Enviando '{}' hacia IP: {}", command, ip);
    
    // Obtenemos un socket disponible aleatorio en el OS
    let socket = std::net::UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| format!("Fallo al abrir socket: {}", e))?;
        
    // Asumimos que el servidor PC va a escuchar los comandos en el puerto 8889
    // (o el puerto que prefieras asignar en el agente del servidor)
    let target = format!("{}:8889", ip);
    
    socket.send_to(command.as_bytes(), target)
        .map_err(|e| format!("Fallo al enviar comando UDP: {}", e))?;
        
    Ok(format!("Comando {} emitido con éxito.", command))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // To be able to launch the app.handle() to a new asynchronous task,
            // we need to clone it. An AppHandle is very cheap to clone.
            let app_handle = app.handle().clone();

            // We spawn an asynchronous task ("green thread") in Tokio to not block the app
            tauri::async_runtime::spawn(async move {
                if let Err(e) = network::listen_for_server(app_handle).await {
                    eprintln!("Error in Discovery (UDP 5000): {}", e);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_audio,
            restart_audio,
            send_media_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
