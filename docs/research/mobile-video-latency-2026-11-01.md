# Mobile video latency investigation — 2.3.2

Scope: Android/iPhone clients, independent of the legacy Mac-to-Windows renderer.

REA 6.5.0 inspected the official Moonlight Android 12.2 non-root APK. Its SHA-256
is `a871f6190365386ef3ac5722a016d4da015ee1555631ec257f6e5dc3f4a45fe0`.
`inspect-artifact` completed a safe static inventory. Detailed Android package /
class inspection was unavailable because the optional jadx-headless-mcp provider
was not installed. No decompilation, runtime timing or private implementation
claim is inferred from that inventory. Local evidence is retained under
`/private/tmp/sanser-mobile-research-232`.

The implementation comparison uses public primary source:

- [Moonlight Android decoder](https://github.com/moonlight-stream/moonlight-android/blob/b48494cb96bff23d8886c4775cc4f39a1075495d/app/src/main/java/com/limelight/binding/video/MediaCodecDecoderRenderer.java):
  lowest-latency output drains decoded buffers, releases older images without
  rendering, then submits the newest buffer with a current presentation time.
- [Moonlight iOS decoder](https://github.com/moonlight-stream/moonlight-ios/blob/02dc9780496eeeac6d01c8bbdccb8b6fe71ef28a/Limelight/Stream/VideoDecoderRenderer.m):
  separates video frame polling/pacing from platform display and supports an
  optional extra frame for smoothness. Sanser does not copy its implementation.
- [Moonlight frame pacing explanation](https://github.com/moonlight-stream/moonlight-docs/wiki/Frequently-Asked-Questions):
  queued images trade latency against smoothing; low latency may replace images
  which have not yet been presented.

Sanser findings: iPhone originally parsed/copy-packed H.264 and enqueued it on the
UI thread. Three queued compressed frames triggered reference-chain recovery.
Android rendered every available decoded buffer and only scheduled one delayed
drain per input; late decoder output could wait until a later input frame.

Changes: ordered/bounded iPhone VideoToolbox decode worker, one decoded-image
mailbox, presentation on main, stream-generation fence; Android newest-output
draining and coalesced repeated polling while frames are in flight. Compressed
P-frames are not freely discarded: overload requires a bounded keyframe recovery.
No extra display-link interval is deliberately introduced in low latency mode.

`bash scripts/test-mobile-video-ios.sh` compiles the production decoder and
generates a real H.264 reference chain with VideoToolbox. It blocks main while
frames arrive, verifies a single pending presentation, rejects an old stream and
checks decoding restarts after reset. It runs on macOS, so iPhone screen scan-out,
thermal behavior and WAN latency remain separate real-device checks.
