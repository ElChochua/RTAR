<img width="722" height="632" alt="image" src="https://github.com/user-attachments/assets/ed7e294d-693e-4f5a-95ad-1cc17001d279" /># RTAR (Real-Time Audio Receiver)

RTAR is a local network (LAN) application built with Tauri and Rust designed to receive ultra-low latency audio from a PC and stream it seamlessly to an Android device.
[Server App RTABC](https://github.com/ElChochua/RTABC)
## Prerequisites

- [Rust and Cargo](https://rustup.rs/) installed.
- [Node.js and npm](https://nodejs.org/) installed.
- Android Studio with Android SDK and NDK installed (required for Android builds).

## How to Run

### 1. Running on Desktop (Testing Environment)

If you want to test the user interface and basic functionality directly on your computer without needing an Android device, you can run the standard desktop version of Tauri:

```bash
npm install     # Install frontend dependencies (only needed the first time)
cargo tauri dev # Launch the application in desktop testing mode
```

### 2. Running on Android via USB (Debug Mode)

To compile the application and run it directly on a physical Android phone:

1. **Enable Developer Options:** Go to your phone's **Settings > About Phone**, and tap the **"Build Number"** 7 times to unlock developer permissions.
2. **Enable USB Debugging:** Navigate back to **Settings > System > Developer Options** and toggle on **"USB Debugging"**.
3. **Connect your device:** Plug your phone into your PC via a USB cable. If a prompt appears on your phone screen asking to "Allow USB debugging", accept it.
4. **Compile and Run:** Execute the following command in your terminal at the root of the project (`RTAR/`):

```bash
cargo tauri android run
```

*Note: The first compilation might take a few minutes as it downloads Android dependencies and compiles the Rust backend for the ARM architecture. Once finished, it will automatically install and open the app on your phone.*

**Screenshots**
<p aling="center">
<img width="400" alt="ss" src="https://github.com/user-attachments/assets/646449b0-f88d-4d10-afcf-5e4477cb0e52" />
<img width="400" alt="image" src="https://github.com/user-attachments/assets/09f3e076-a88e-4ea4-b342-059fefecaaa4" />
<img width="400" alt="image" src="https://github.com/user-attachments/assets/480bf852-292c-48a0-aa8c-1481aabefdd3" />


</p>
