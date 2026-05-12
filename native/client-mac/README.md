# Sanser Native macOS Client

This is the first Phase 5 prototype for the long-term Sunshine/Parsec-style client path.

Current scope:

- VideoToolbox hardware decode capability probe
- Metal render test window
- Native mouse, keyboard, scroll event logging from the Metal view
- Native clipboard read/write probe

Not included yet:

- Realtime H.264/H.265/AV1 packet decode
- CVPixelBuffer-to-Metal texture render path
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
