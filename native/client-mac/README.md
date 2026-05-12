# Sanser Native macOS Client

This is the first Phase 5 prototype for the long-term Sunshine/Parsec-style client path.

Current scope:

- VideoToolbox hardware decode capability probe
- SNV1 H.264 packet file parser
- VideoToolbox decode test from Windows native host `.snv` captures
- TCP SNV1 receiver with VideoToolbox decode and Metal render
- Metal render test window
- Native mouse, keyboard, scroll event logging from the Metal view
- Native clipboard read/write probe

Not included yet:

- Realtime H.264/H.265/AV1 network packet decode
- Jitter buffer
- Audio decode/playback
- Integration with the Electron client stream view

## Build

From the repository root on macOS:

```bash
npm run native:client-mac:configure
npm run native:client-mac:build
```

## Probe

```bash
npm run native:client-mac:probe
```

Expected output includes the Metal device and whether VideoToolbox reports hardware decode support for H.264, H.265/HEVC, and AV1.

## Decode SNV1 H.264 Packet File

After generating an `.snv` file on Windows:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --frames 180 --fps 60 --interval-ms 0 --bitrate 28000000 --packet-file native-captures\capture_h264.snv
```

Copy `capture_h264.snv` to the Mac, then decode-test it:

```bash
npm run native:client-mac:decode-snv -- /path/to/capture_h264.snv
```

The output reports parsed packets, keyframes, NAL units, submitted frames, decoded frames, and decode errors. `decodedFrames` greater than zero means VideoToolbox accepted the Windows native packet stream.

## TCP SNV1 Listener

Start the macOS native client as a TCP receiver:

```bash
npm run native:client-mac:listen-snv -- 7777 --max-packets 180
```

Then start the Windows native host with the Mac Tailscale IP:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --frames 180 --fps 60 --interval-ms 0 --bitrate 28000000 --tcp-connect MAC_TAILSCALE_IP:7777
```

The Mac command decodes packets as they arrive and prints the same decode summary. This is Phase 6C's first network transport: TCP over Tailscale, before RTP/UDP/QUIC.

## TCP SNV1 Metal Render

Start the macOS native client with a Metal render window:

```bash
npm run native:client-mac:listen-render-snv -- 7777 --max-packets 180
```

Then start the Windows native host:

```powershell
.\native\host-win\build\Release\sanser-native-host.exe --encode-pipe h264 --frames 180 --fps 60 --interval-ms 0 --bitrate 28000000 --tcp-connect MAC_TAILSCALE_IP:7777
```

Decoded `CVPixelBuffer` frames are submitted to a Metal renderer and displayed as NV12 textures.

## Metal Render Test

```bash
./native/client-mac/build/sanser-native-client --metal-test --seconds 5
```

The window renders directly with Metal. Mouse, keyboard, and scroll events are logged to stdout as `SNINPUT` JSON lines. Use `--seconds 0` to keep the window open.

## Clipboard Probe

```bash
./native/client-mac/build/sanser-native-client --clipboard-write "hello from Sanser"
./native/client-mac/build/sanser-native-client --clipboard-read
```

This validates the native clipboard bridge that will later synchronize clipboard text between the native client and host.
