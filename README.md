# GameRemote

A single-app Parsec-like prototype for low-latency remote game streaming.

## What is included

- Login and register
- My Computers list
- Host mode with online status
- Client connect flow
- WebRTC peer-to-peer screen streaming
- 720p, 1080p, 1440p, FPS, bitrate, and codec preference
- Remote input data channel
- Reliable input channel plus realtime mouse/transport feedback channel
- Live FPS, bitrate, jitter, packet loss, and RTT stats
- WebRTC adaptive bitrate prototype

## Run

```bash
npm run dev
```

Open `http://127.0.0.1:5174`.

For two computers on the same LAN, run the app on the host machine with:

```bash
HOST=0.0.0.0 npm run dev
```

Then open `http://HOST_MACHINE_IP:5174` on the client machine.

## Desktop app

Run the Electron desktop app:

```bash
npm run desktop
```

Build a portable Windows `.exe`:

```bash
npm run dist:win
```

Build a macOS `.zip` containing `GameRemote.app`:

```bash
npm run dist:mac
```

Build both desktop versions:

```bash
npm run dist:both
```

The Windows output is written to `dist/GameRemote-Windows-0.1.0-portable-x64.exe`.
The macOS output is written to `dist/GameRemote-macOS-0.1.0-arm64.zip`.

## Same-account computers

For automatic computer discovery across macOS and Windows, both apps must use the same server URL in `Settings > Network`. The embedded local server works for single-machine testing; use a shared LAN or cloud relay server when connecting two different computers.

The desktop app starts an embedded relay on port `5174` when the port is free. On a LAN, open the Windows app, then set the Mac app's server URL to:

```text
http://WINDOWS_LAN_IP:5174
```

After both apps login with the same account on that server, the Windows computer appears automatically in `Computers`.

## Tailscale mode

For free cross-network testing without router port forwarding, install Tailscale on both computers and sign in to the same tailnet:

```bash
npm run tailscale:install
```

The desktop app also checks this automatically at startup when `NETWORK_MODE=tailscale`; if Tailscale is missing, it offers to install it using Homebrew on macOS or winget on Windows. After install, Sanser starts Tailscale and opens the login flow if the machine is not signed in yet.

Then set this in `.env` on both computers:

```text
NETWORK_MODE=tailscale
ICE_TRANSPORT_POLICY=all
```

Restart Sanser on both machines. Tailscale must stay connected while streaming. If Windows uses a packaged `.exe`, rebuild and copy the latest app after code changes.

## Smoothness notes

This version uses Electron/WebRTC screen capture. Default quality is tuned for a sharper low-latency Tailscale session at `1080p`, `60 FPS`, and about `28 Mbps` with VP8. The WebRTC Adaptive transport mode uses client feedback to reduce bitrate when RTT, jitter, packet loss, or FPS pressure rises, then slowly recovers toward the requested bitrate. If it still stutters, lower bitrate to `16-20 Mbps` or FPS to `30`; if text is still blurry and the network is stable, raise bitrate to `35-45 Mbps`.

Parsec is smoother because it uses native low-level capture, dedicated GPU encoder control, a custom low-latency transport, adaptive congestion control, and a native input driver. Sanser is a WebRTC/Electron prototype, so true Parsec-grade game feel still needs native capture, GPU encode control, deeper input injection, and more mature network adaptation.

## Native Engine Roadmap

The first native Windows host prototype lives in `native/host-win`. It uses Windows Desktop Duplication API to capture frames without `getDisplayMedia`.

On Windows with Visual Studio Build Tools and CMake:

```powershell
npm run native:host-win:configure
npm run native:host-win:build
.\native\host-win\build\Release\sanser-native-host.exe --frames 5 --interval-ms 100 --output-dir native-captures
```

The prototype writes BMP frames for validation, supports `--pipe --fps 60` to stream BGRA frames to stdout, and has a Media Foundation H.264 file-encode prototype. The app transport currently remains WebRTC with adaptive bitrate and split data channels. The later transport step is moving native encoded packets onto RTP first, then a custom UDP/QUIC transport with a jitter buffer.

## Native macOS Client Prototype

The first Phase 5 native client prototype lives in `native/client-mac`. It probes VideoToolbox hardware decode support, opens a Metal render test window, logs native input events, and validates native clipboard read/write.

On macOS:

```bash
npm run native:client-mac:configure
npm run native:client-mac:build
npm run native:client-mac:probe
./native/client-mac/build/sanser-native-client --metal-test --seconds 5
```

This does not replace the Electron/WebRTC client view yet. The next native-client step is receiving realtime encoded frames from the Windows native host, decoding them with VideoToolbox, then rendering decoded `CVPixelBuffer` frames as Metal textures.
