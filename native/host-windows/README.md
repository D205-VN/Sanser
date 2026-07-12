# Sanser Windows host engine

This C++20 target contains the Windows media/input implementation being migrated into Sanser 2.0.2.

Implemented and retained from the native engine:

- Desktop Duplication/D3D11 capture fallback.
- Media Foundation H.264/HEVC encoding and bitrate/keyframe control.
- WASAPI loopback audio.
- Native keyboard, mouse and gamepad injection.
- Bounded UDP reassembly/retransmission feedback primitives.
- Capture recovery and rotated-display handling.

Sanser 2 additions:

- Target name `sanser-host-windows`.
- Shared, tested `native/protocol` SNV2 header/authentication library.
- Product/protocol compile-time version `2.0.2` / `2`.

The migrated legacy stream loop is not considered SNV2-capable until its packet lanes and handshake use the shared protocol library end-to-end. The Tauri shell must not advertise or package this sidecar as `nativeSnv2=true` before that capability probe and interoperability tests pass.

The sidecar reports the existing authenticated/encrypted path separately as
`nativeDirect=true`. For a negotiated base port it sends video to the macOS
client on `base`, always establishes authenticated control on `base+1`, and
optionally sends audio on `base+2`. `--disable-input` keeps control/rekey/stats
active while rejecting remote keyboard, pointer, clipboard and gamepad events.

## Build

From a Visual Studio 2022 Developer PowerShell:

```powershell
npm run native:host-windows:configure
npm run native:host-windows:build
```

Run `ctest --test-dir native/protocol/build --output-on-failure` for the portable wire codec. Windows release verification must cover H.264/HEVC, GPU vendors, display mode/access-loss, portrait outputs, NACK loss, audio, input reset, and long sessions.
