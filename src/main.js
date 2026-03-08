const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let discoveryStatusEl;
let serverPanelEl;
let serverIpDisplayEl;
let connectBtnEl;
let reconnectBtnEl;

let activeServerIp = null;

async function setupDiscoveryListener() {
  // Escuchamos el evento 'server_found' que emite nuestro backend en Rust
  await listen("server_found", (event) => {
    activeServerIp = event.payload;
    console.log("Servidor encontrado desde Rust:", activeServerIp);

    // Actualizamos la UI
    if (discoveryStatusEl && serverPanelEl && serverIpDisplayEl) {
      discoveryStatusEl.style.display = "none";
      serverPanelEl.style.display = "block";
      serverIpDisplayEl.textContent = `IP: ${activeServerIp}`;
    }
  });
}

window.addEventListener("DOMContentLoaded", () => {
  discoveryStatusEl = document.querySelector("#discovery-status");
  serverPanelEl = document.querySelector("#server-panel");
  serverIpDisplayEl = document.querySelector("#server-ip-display");
  connectBtnEl = document.querySelector("#connect-btn");
  reconnectBtnEl = document.querySelector("#reconnect-btn");

  if (connectBtnEl) {
    connectBtnEl.addEventListener("click", () => {
      if (activeServerIp) {
        console.log("Iniciando conexión de audio hacia:", activeServerIp);
        // Aquí llamamos al comando de Tauri para inicializar Cpal 
        invoke("start_audio", { ip: activeServerIp })
          .then((msg) => {
            console.log(msg);
            connectBtnEl.textContent = "¡Conectado! Escuchando audio...";
            connectBtnEl.disabled = true;
          })
          .catch((err) => {
            console.error(err);
            alert("Error al iniciar audio: " + err);
          });
      }
    });
  }

  if (reconnectBtnEl) {
    reconnectBtnEl.addEventListener("click", () => {
      console.log("Reiniciando App para Hot-Swap de Audio...");

      // 1. Matar el Streaming Zombie en Rust usando el nuevo comando
      invoke("restart_audio")
        .then(() => {
          // 2. Reseteo limpio de la interfaz de usuario Web
          window.location.reload();
        })
        .catch((err) => {
          console.error("Error destruyendo audio de fondo: ", err);
          window.location.reload(); // Recargar de todos modos
        });
    });
  }

  // Inicializamos el listener asíncrono
  setupDiscoveryListener();
});
