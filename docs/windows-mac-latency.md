# Windows client → Mac host latency work

This is the SNV2 path: ScreenCaptureKit → VideoToolbox → authenticated UDP →
Windows Media Foundation → NV12 conversion → GDI presentation. It is independent
of the legacy Mac-client/Windows-host baseline documented for 2.2.1.

## Changes in 2.2.8

The capture callback retains the latest raw picture and immediately returns.
A worker keeps one pending newest raw capture while encoding and publishing one
picture at a time. It completes that encode before admitting another picture,
so an encoder or publisher stall cannot build up seconds of raw-picture work
inside VideoToolbox. This also preserves the final pending picture when the
desktop becomes idle. Keyframe refreshes reuse the retained capture.

`RealTime` alone is a recommendation. The encoder requests a bounded compression
window where supported, and `VTCompressionSessionCompleteFrames` enforces the
one-picture submission boundary even when the optional window property is not
supported. See Apple's [RealTime](https://developer.apple.com/documentation/VideoToolbox/kVTCompressionPropertyKey_RealTime),
[MaxFrameDelayCount](https://developer.apple.com/documentation/videotoolbox/kvtcompressionpropertykey_maxframedelaycount)
and [compression session](https://developer.apple.com/documentation/videotoolbox/vtcompressionsession-api-collection)
documentation. Completing each picture limits encoder pipelining; throughput on
the affected Mac must be tested at the requested resolution and FPS.

Windows draws a signaled picture synchronously instead of leaving `WM_PAINT`
behind input messages. The normal message loop yields after 64 messages or 2 ms,
whichever occurs first. Existing resize-modal-loop painting remains available.

SNV2 decrypt/authenticate originally opened AES and HMAC providers per packet.
Windows now uses one provider per thread, with fresh keyed hash/AES/IV state for
every call. Microsoft explicitly recommends caching these expensive
[algorithm providers](https://learn.microsoft.com/en-us/windows/win32/api/bcrypt/nf-bcrypt-bcryptopenalgorithmprovider).
This changes setup overhead, not the wire format or security checks.

## Measurements saved automatically

`SNV2_HOST_TIMING` reports cumulative captured/submitted/completed/drop counts,
`encodeQueueAvgMs` and `encodeAndPublishAvgMs`. A large difference between raw
captures and submissions can mean superseded captures were skipped; it is not
evidence that encoded reference frames were lost. Encode-and-publish includes
the native output callback/packet preparation, not just the hardware encoder.

`SNV2_VIDEO_HEALTH` reports decoded/converted/rendered counts, encoded-queue drops,
decode errors and these cumulative local averages:

- `decodeWorkAvgMs`: one compressed input submission and available decoder output,
  including cold initialization in the first call. MF can output multiple or no
  pictures for a call; this is not strictly hardware time per decoded picture.
- `conversionAvgMs`: NV12-to-BGRA work.
- `paintAvgMs`: painting the first presentation of each new picture.
- `decodedToPaintAvgMs`: decoded-buffer creation through copying, conversion,
  scheduling and first presentation. Unchanged idle repaints are excluded.

No sample is emitted for an average before that stage has a sample. These
durations do not include cross-machine capture/network latency or display scan-out
and must not be added as if all stages were disjoint. They are not a new Wire RTT
measurement. Existing legacy Wire/echo measurements are unchanged.

The automatic report preserves these allowlisted numeric metrics and records
`wireProtocol`, `requestedWidth`, `requestedHeight` and `requestedCodec` in addition
to its existing FPS/bitrate/mode. Native input telemetry retains event **counts**
only. Raw keys, text, screen pixels and session credentials remain excluded.

## Validation and remaining physical test

The real Mac encoder backpressure regression blocks its publisher, submits 101
new captures and verifies only the original reference and the final newest picture
are encoded, delivered and decoded. Capture submissions must return without
waiting on the blocked publisher. The last picture must arrive even without any
additional desktop change after the stall ends.

The Windows message regression leaves posted messages queued, paints a new
picture anyway and checks that ordinary input/view-only rules still hold.
Both original and realtime 1846×1200 VideoToolbox fixtures exercise Windows MF.
Crypto tests retain golden authentication vectors, padding/empty-control tests,
tamper rejection and interleaved sessions on four threads.

On the affected machines, test ordinary Windows → Mac control for at least one
minute, with group sharing and reverse-direction streaming stopped. Keep route,
VPN/adapters and stream settings fixed. Compare idle typing/clicks and active
window movement; export Diagnostics on **both computers** while the delay occurs.
No physical three-second-lag fix or 10–15 ms target is claimed by unit tests.
