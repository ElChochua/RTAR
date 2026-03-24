const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

let discoveryStatusEl;
let serverPanelEl;
let serverIpDisplayEl;
let connectBtnEl;
let reconnectBtnEl;
let mediaControlsEl;
let btnPrevEl;
let btnPlayPauseEl;
let btnNextEl;

let activeServerIp = null;
let wakeLock = null;

async function setupDiscoveryListener() {
  // We listen to the 'server_found' event emitted by our backend in Rust
  await listen("server_found", (event) => {
    activeServerIp = event.payload;
    console.log("Server discovered from Rust:", activeServerIp);

    // Update the UI
    if (discoveryStatusEl && serverPanelEl && serverIpDisplayEl) {
      discoveryStatusEl.style.display = "none";
      serverPanelEl.style.display = "block";
      serverIpDisplayEl.textContent = `IP: ${activeServerIp}`;
    }
  });
}

// 1. Avoid web Suspention
// Android Doze Mode usually kills the network timer. A wakelock can prolong the connection.
async function requestWakeLock() {
  try {
    if ('wakeLock' in navigator) {
      wakeLock = await navigator.wakeLock.request('screen');
      console.log('Web Wakelock activated. Preventing the network from sleeping.');
      wakeLock.addEventListener('release', () => {
        console.log('WakeLock was released (e.g. manual screen off)');
      });
    }
  } catch (err) {
    console.warn(`Error WakeLock: ${err.name}, ${err.message}`);
  }
}

// 2. Comandos Multimedia y Audífonos Bluetooth (AVRCP)
function setupMediaEvents(ip) {
  if (mediaControlsEl) mediaControlsEl.style.display = 'block';

  // Function proxy to Rust
  const sendCmd = (cmd) => {
    console.log("Sending AVRCP/Multimedia command =>", cmd);
    invoke("send_media_command", { ip, command: cmd })
      .catch(e => console.error(e));
  };

  // HTML Screen Buttons
  if (btnPrevEl) btnPrevEl.onclick = () => sendCmd("prev");
  if (btnPlayPauseEl) btnPlayPauseEl.onclick = () => sendCmd("play_pause");
  if (btnNextEl) btnNextEl.onclick = () => sendCmd("next");

  // Botones Físicos de Audífonos Bluetooth interceptados
  if ('mediaSession' in navigator) {
    navigator.mediaSession.metadata = new MediaMetadata({
      title: 'Local Audio Stream',
      artist: 'Transmitiendo desde PC',
      album: 'RTABC',
    });

    navigator.mediaSession.setActionHandler('play', () => sendCmd('play_pause'));
    navigator.mediaSession.setActionHandler('pause', () => sendCmd('play_pause'));
    navigator.mediaSession.setActionHandler('previoustrack', () => sendCmd('prev'));
    navigator.mediaSession.setActionHandler('nexttrack', () => sendCmd('next'));
  }
}

window.addEventListener("DOMContentLoaded", () => {
  discoveryStatusEl = document.querySelector("#discovery-status");
  serverPanelEl = document.querySelector("#server-panel");
  serverIpDisplayEl = document.querySelector("#server-ip-display");
  connectBtnEl = document.querySelector("#connect-btn");
  reconnectBtnEl = document.querySelector("#reconnect-btn");
  mediaControlsEl = document.querySelector("#media-controls");
  btnPrevEl = document.querySelector("#btn-prev");
  btnPlayPauseEl = document.querySelector("#btn-playpause");
  btnNextEl = document.querySelector("#btn-next");

  if (connectBtnEl) {
    connectBtnEl.addEventListener("click", () => {
      if (activeServerIp) {
        console.log("Starting audio connection to:", activeServerIp);
        // Here we call the Tauri command to initialize Cpal 
        invoke("start_audio", { ip: activeServerIp })
          .then((msg) => {
            console.log(msg);
            connectBtnEl.textContent = "¡Connected! Listening to audio...";
            connectBtnEl.disabled = true;

            // Initialize physical interceptors and web buttons
            setupMediaEvents(activeServerIp);
            requestWakeLock();
          })
          .catch((err) => {
            console.error(err);
            alert("Error starting audio: " + err);
          });
      }
    });
  }

  if (reconnectBtnEl) {
    reconnectBtnEl.addEventListener("click", () => {
      console.log("Restarting App for Audio Hot-Swap...");

      // 1. Kill the Zombie Streaming in Rust using the new command
      invoke("restart_audio")
        .then(() => {
          // 2. Clean reset of the Web user interface
          window.location.reload();
        })
        .catch((err) => {
          console.error("Error destroying background audio: ", err);
          window.location.reload(); // Reload anyway
        });
    });
  }

  // Initialize the asynchronous listener
  setupDiscoveryListener();
});
