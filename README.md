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
- Native Windows capture with H.264, HEVC, or Auto codec selection and macOS VideoToolbox/Metal render
- Native UDP video, audio, input, gamepad, retransmit, and adaptive feedback path

## Run

Create a local environment file first and set a PostgreSQL connection string:

```bash
cp .env.example .env
```

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

For automatic computer discovery across macOS and Windows, both apps must use the same server URL in `Settings > Network`. The embedded local server works for single-machine testing; use a shared LAN or cloud signaling server when connecting two different computers, plus TURN only when WebRTC relay mode is required.

The desktop app starts an embedded signaling/API server on port `5174` when the port is free. On a LAN, open the Windows app, then set the Mac app's server URL to:

```text
http://WINDOWS_LAN_IP:5174
```

After both apps login with the same account on that server, the Windows computer appears automatically in `Computers`.

## Network modes without Tailscale

`NETWORK_MODE=direct` uses direct WebRTC candidates and STUN. It is the simplest mode for LAN use and for networks where NAT/firewall allows a peer-to-peer route:

```text
NETWORK_MODE=direct
STUN_URLS=stun:stun.l.google.com:19302
```

Direct mode forces WebRTC `iceTransportPolicy=all` and does not add configured TURN servers. Native SNV also works on a LAN or over an explicitly routable address, but its UDP video/audio and TCP control ports do not traverse STUN or TURN; cross-NAT native use requires routing or port forwarding.

`NETWORK_MODE=relay` routes WebRTC media through a TURN server and does not require Tailscale. Configure at least one TURN URL and credentials on the shared app server:

```text
NETWORK_MODE=relay
TURN_URLS=turn:turn.example.com:3478?transport=udp,turns:turn.example.com:5349?transport=tcp
TURN_USERNAME=replace-me
TURN_CREDENTIAL=replace-me
```

Relay mode forces `iceTransportPolicy=relay`. The desktop UI automatically switches Native SNV to WebRTC Adaptive in this mode because the native UDP protocol does not use TURN. Prefer both UDP and TLS/TCP TURN listeners so restrictive networks still have a route. Do not put long-lived production TURN secrets in a committed `.env` file.

`NETWORK_MODE=default` remains available for backward compatibility and honors an explicit `ICE_TRANSPORT_POLICY`; otherwise it defaults to `all`, trying direct candidates first and keeping TURN as fallback. `NETWORK_MODE=tailscale` is also retained as an optional legacy mode. Only that legacy mode starts or offers to install Tailscale, and `TAILSCALE_USE_STUN=0` keeps its WebRTC ICE list empty.

## Smoothness notes

Native SNV is the desktop default outside relay mode. Auto quality selects `1080p60 / 28 Mbps` on wired LAN, `900p45 / 12 Mbps` on Wi-Fi, or `720p30 / 6 Mbps` on Internet/Relay routes. These are real encoder bounds: larger and ultrawide desktops are aspect-fitted and converted directly to the target NV12 size instead of encoding every source pixel at a low bitrate. The old stored `tailscale` profile is migrated to `internet` by the UI and remains accepted by the server for compatibility.

Native codec `Auto` preserves the requested `auto` metadata while resolving the codec for execution: LAN prefers H.264, while Wi-Fi and Internet profiles prefer HEVC. If the Windows encoder cannot start HEVC, the native host falls back to H.264. Choose explicit H.264 for maximum compatibility or explicit HEVC when you have already validated both machines.

Leave Native Client IP blank so direct/LAN discovery can select the appropriate interface. Use Manual only when you need to override the automatic resolution, FPS, or bitrate.

Native audio negotiates PCM16 with the current Mac client, cutting audio payload by about half compared with the legacy float32 wire format while retaining float32 compatibility for older clients. WebRTC Adaptive remains the cross-platform fallback and reduces bitrate under RTT, jitter, packet-loss, or FPS pressure. Hardware/driver behavior varies, so validate H.264 and HEVC on the actual Windows machine; use H.264 first when compatibility matters.

## Native Engine

The native Windows host lives in `native/host-win`. It uses Windows Desktop Duplication API to capture frames without `getDisplayMedia`, recovers after display access loss, and handles rotated outputs.

On Windows with Visual Studio Build Tools and CMake:

```powershell
npm run native:host-win:configure
npm run native:host-win:build
.\native\host-win\build\Release\sanser-native-host.exe --frames 5 --interval-ms 100 --output-dir native-captures
```

The host writes BMP frames for validation, supports `--pipe --fps 60` to stream BGRA frames to stdout, and writes realtime-style H.264/HEVC `SNV1` packets:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --frames 180 --fps 60 --interval-ms 0 --bitrate 28000000 --packet-file native-captures\capture_h264.snv
npm run native:host-win:inspect-snv -- native-captures\capture_h264.snv
```

The native host can stream SNV1 over TCP for manual diagnostics; the desktop flow uses UDP video plus dedicated control and audio ports:

```bash
npm run native:client-mac:listen-snv -- 7777 --max-packets 180
```

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --frames 180 --fps 60 --interval-ms 0 --bitrate 28000000 --tcp-connect <MAC_IP>:7777
```

WebRTC remains available as a fallback transport.

## Native macOS Client

The native macOS client lives in `native/client-mac`. It supports VideoToolbox H.264/HEVC decode, Metal render, UDP reassembly/jitter/NACK recovery, negotiated float32/PCM16 audio playback, and the native input/gamepad backchannel.

On macOS:

```bash
npm run native:client-mac:configure
npm run native:client-mac:build
npm run native:client-mac:probe
npm run native:client-mac:decode-snv -- /path/to/capture_h264.snv
npm run native:client-mac:listen-snv -- 7777 --max-packets 180
npm run native:client-mac:listen-render-snv -- 7777 --max-packets 180
./native/client-mac/build/sanser-native-client --metal-test --seconds 5
```

The desktop app launches this path when Settings -> Network -> Transport is set to `Native SNV`. The Mac client opens its Metal renderer and listeners, sends the selected endpoint through the app server, and the Windows host starts the native capture/encode process. Electron remains the account/device/control shell; native code owns capture, encode, render, audio, and the SNINPUT backchannel.

## Checks

```bash
npm run check
npm test
npm run native:client-mac:build
```

The PostgreSQL integration test is skipped unless `TEST_DATABASE_URL` is set. Use a disposable test database for CI.

### Manual native diagnostics

For manual testing, keep using:

```bash
npm run native:client-mac:listen-render-snv -- 7777 --log-input
```

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --fps 60 --interval-ms 0 --bitrate 28000000 --tcp-connect <MAC_IP>:7777
```
