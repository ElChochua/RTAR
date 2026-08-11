# Arquitectura Android de RTAR

La arquitectura canónica del producto vive en `RTABC/docs`. Este documento fija responsabilidades específicas de RTAR.

## Responsabilidades

- Tauri HTML/CSS: selección de modo, dispositivo, calidad y estado.
- Foreground service nativo: lifecycle, notificación, audio de fondo y proyección.
- Rust: protocolo, transporte, Opus, buffers, métricas y state machine.
- `MediaProjection`: captura de pantalla autorizada por usuario.
- `AudioRecord` con `AudioPlaybackCaptureConfiguration`: audio de aplicaciones que permitan captura.
- `MediaCodec`: HEVC hardware con surface input, sin copiar frames por JavaScript.

## Servicios

### PlaybackForegroundService

Se inicia mediante acción explícita del usuario. Posee sesión receptora, salida de audio y `MediaSession`. Permanece visible mediante notificación mientras reproduce. Detener desde UI o notificación cancela tareas y libera audio.

### ProjectionForegroundService

Se inicia después de obtener consentimiento `MediaProjection`. Posee `VirtualDisplay`, surface del encoder, `AudioRecord` y sesión emisora. `MediaProjection.Callback.onStop()` libera todos los recursos e informa a UI.

## Restricciones

- Bloquear pantalla puede terminar proyección por diseño de Android.
- No todo audio es capturable; DRM y políticas de cada aplicación se respetan.
- Wake lock de pantalla no sustituye foreground service.
- No se guardan `cpal::Stream` mediante raw pointers globales.
- Buffers son fijos y priorizan descartar contenido viejo.

## Compatibilidad

`src-tauri/src/protocol.rs` debe mantener el vector dorado de `RTABC/src/protocol.rs`. Cualquier diferencia requiere cambio explícito de versión.

## Audio implementado en Stage 2

- Opus estéreo, 48 kHz y cuadros de 10 ms.
- Precarga de red de 20 ms y máximo de 80 ms; la cola PCM local se limita a 50 ms.
- Reordenamiento, rechazo de duplicados/tardíos y PLC/FEC ante pérdida.
- Una discontinuidad o cambio de stream limpia estado viejo antes de reproducir.
