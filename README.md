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
- Live FPS, bitrate, packet loss, and RTT stats

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
STUN_URLS=stun:stun.l.google.com:19302
NETWORK_MODE=tailscale
ICE_TRANSPORT_POLICY=all
```

Restart Sanser on both machines. Tailscale must stay connected while streaming. If Windows uses a packaged `.exe`, rebuild and copy the latest app after code changes.

## Smoothness notes

This version uses WebRTC P2P with browser hardware acceleration where available. It is the correct base for smooth streaming without Sunshine/Moonlight, but true Parsec-grade game feel still needs native capture, GPU encode control, native input injection, and TURN/relay infrastructure for hard NAT cases.
