# Sanser 2 performance contract

Performance claims are recorded only after measurement on named hardware. Until then, the values below are budgets and invariants, not benchmark results.

## Bounded realtime state

| Queue | Policy |
| --- | --- |
| Capture/encode | 1–3 frames; drop oldest when encoder is late |
| Render | newest ready frame wins; expired frames drop |
| Mouse/gamepad | one coalesced latest state per device |
| Keys/buttons | bounded reliable ordered lane; disconnect resets state |
| Audio | bounded by packets, bytes, and target milliseconds |
| Reassembly | bounded by session, bytes, fragment count, and age |
| Retransmission | bounded by media generation and playout deadline |
| Logs/diagnostics | ring buffers with sanitized rotation/export |

## Pipeline timing

Diagnostics record capture, conversion, encode, send queue, network, reassembly, decode, render, audio buffer, and input capture/network/injection latency separately. Adaptive quality uses measured RTT, loss, jitter, encoder/decoder pressure, and frame drops.

## Native principles

Windows capture and color conversion should remain on D3D11 textures and use hardware encoding when the driver path is verified. macOS decode outputs `CVPixelBuffer` directly to Metal. No native frame crosses JavaScript. Allocation and CPU copies in the per-frame path must be measured and eliminated or reused.

## Benchmark status

Startup time, idle RAM/CPU, artifact size, FPS, bitrate, RTT, input latency, encode latency, and decode latency are currently **not yet measured for the complete Sanser 2 application**. Release documentation must not substitute legacy Electron measurements.
