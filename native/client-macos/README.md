# Sanser macOS client engine

This Objective-C++20 target contains the macOS media/input implementation being migrated into Sanser 2.0.0.

Implemented native components:

- VideoToolbox H.264/HEVC decode.
- Direct Metal rendering from decoded pixel buffers.
- Bounded packet reassembly, jitter, NACK and keyframe recovery.
- CoreAudio/AudioQueue playback with negotiated PCM16 compatibility.
- Native keyboard, mouse and GameController capture/feedback.
- Authenticated media/control primitives and listener allocation limits.

Sanser 2 additions:

- Target name `sanser-client-macos`.
- Shared, tested `native/protocol` SNV2 header/authentication library.
- Product/protocol compile-time version `2.0.0` / `2`.

The legacy listener loop remains migration reference and is not reported as SNV2-capable until it consumes the shared header, session, replay and priority-lane implementation end-to-end. The Tauri shell keeps that capability unavailable meanwhile.

## Build and probe

```bash
npm run native:client-macos:configure
npm run native:client-macos:build
npm run native:client-macos:probe
```

The full release matrix must additionally verify authenticated SNV2 interoperability with the Windows host, audio hot-plug, input release on crash/disconnect, packet loss, decoder recovery, and long-session memory bounds.
