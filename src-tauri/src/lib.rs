mod network;
mod audio;
use lazy_static::lazy_static;
use std::sync::Mutex;

// Un raw pointer (*mut) no implementa Send/Sync por defecto (seguridad de Rust para hilos).
// Hacemos un contenedor manual jurándole a Rust que nosotros controlamos su acceso vía Mutex.
struct StreamHandle(*mut cpal::Stream);
unsafe impl Send for StreamHandle {}
unsafe impl Sync for StreamHandle {}

lazy_static! {
    static ref GLOBAL_AUDIO_STREAM: Mutex<Option<StreamHandle>> = Mutex::new(None);
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn start_audio(ip: &str) -> Result<String, String> {
    println!("Frontend solicitó conectar a la IP de audio: {}", ip);
    
    // Tratamos de inicializar CPAL y lo guardamos
    match audio::setup_audio_stream() {
        Ok(stream) => {
            // Empaquetamos en el Heap (Box), lo convertimos en Raw Pointer y lo guardamos en la global
            let stream_ptr = Box::into_raw(Box::new(stream));
            let mut global_stream = GLOBAL_AUDIO_STREAM.lock().unwrap();
            
            // Si por alguna razón había uno vivo, lo matamos (para evitar memory leaks)
            if let Some(old_handle) = global_stream.take() {
                unsafe {
                    drop(Box::from_raw(old_handle.0));
                }
            }

            *global_stream = Some(StreamHandle(stream_ptr));
            Ok(format!("Stream de audio local inicializado para {}", ip))
        },
        Err(e) => Err(format!("Fallo al inicializar CPAL: {}", e))
    }
}

/// Destruye el hilo crasheado ("The requested device is no longer available")
/// Limpia la memoria y corta el Ring Buffer actual para que se pueda reiniciar
#[tauri::command]
fn restart_audio() -> Result<String, String> {
    println!("Frontend solicitó reiniciar el subsistema de Audio (Hot-Swap)...");
    
    let mut global_stream = GLOBAL_AUDIO_STREAM.lock().unwrap();
    if let Some(handle) = global_stream.take() {
        unsafe {
            // Al reconstruir el Box y dejar que se caiga fuera de scope, CPAL ejecuta
            // el drop() interno, soltando el 'dead IAudioTrack' de Android.
            drop(Box::from_raw(handle.0));
        }
    }
    Ok("Servicio de audio detenido exitosamente.".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Para poder lanzar el app.handle() a una nueva tarea asíncrona,
            // necesitamos clonarlo. Un AppHandle es muy barato de clonar.
            let app_handle = app.handle().clone();

            // Levantamos una tarea asíncrona ("hilo verde") en Tokio para no bloquear la app
            tauri::async_runtime::spawn(async move {
                if let Err(e) = network::listen_for_server(app_handle).await {
                    eprintln!("Error en el Discovery (UDP 5000): {}", e);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_audio,
            restart_audio
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
