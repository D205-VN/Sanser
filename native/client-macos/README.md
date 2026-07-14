# Sanser macOS client engine

This Objective-C++20 target contains the macOS media/input implementation used by Sanser 2.0.8.

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
- Product/protocol compile-time version `2.0.8` / `2`.

The sidecar reports `nativeSnv2=true` and `nativeDirect=true` after its probe
validates the compiled protocol/version and decoder implementation. It listens for
authenticated/encrypted video on the negotiated base port, control on `base+1`
and optional audio on `base+2`. It rejects unauthenticated media whenever a
session credential is present. The same UDP interface can target the local
encrypted relay bridge when Direct UDP is unavailable.

## Build and probe

```bash
npm run native:client-macos:configure
npm run native:client-macos:build
npm run native:client-macos:probe
```

The full release matrix must additionally verify authenticated SNV2 interoperability with the Windows host, audio hot-plug, input release on crash/disconnect, packet loss, decoder recovery, and long-session memory bounds.
