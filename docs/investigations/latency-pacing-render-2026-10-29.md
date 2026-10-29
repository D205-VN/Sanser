# Bounded pacing groups and GPU completion wakeups

Scope: the Ultra legacy Mac viewer → Windows host path shown in the supplied
2.2.25 overlays. This is source work on Version-2.0.0, subsequent to the 2.2.25
release. It does not replace the installed applications or publish a release.

## Measurement interpretation

The two supplied snapshots show wire RTT around 8.9–11.5 ms, pacing wait around
4.5–4.6 ms, decode around 2.2–2.5 ms and decoded-frame queue around 1.7–2.5 ms.
Control queues and host send lock are near zero. Capture wait is the wait for
the next captured picture, separately measured from capture work.

`Send total` already includes pacing and `Pacing wait` includes timer overshoot.
These values must not be added together as independent latency stages. The
pacer uses absolute deadlines in Ultra, so a sum of small timer overshoots is
not automatically the same amount of end-to-end frame delay. RTT, game frame
generation and display scan-out also prevent this overlay from proving total
input-to-photon latency.

## REA reference provenance

REA 6.5.0 imported three selected public source files with
`import-reference-source --detail summary`, authority `historical-reference`.
Every local Git blob hash was verified against the GitHub fetch result.
Manifest: `hsm_56de2f7f919cf7d7607f6ff4b0346863b104c8a7285d45407100d6334329d956`.
Graph SHA-256: `10b89d9ea52ee599c73fa485209e46e6f3c0af0a745a873943eb44c16fd174cc`.
This is source review with provenance, not a runtime benchmark or a private
Parsec binary reconstruction. No competitor source is copied into production.

| Reference | Commit | Git blob |
| --- | --- | --- |
| Sunshine `src/stream.cpp` | `a679b059096bcdd502e2192ce4f6a3c3732fbaae` | `2f2db0c2fc97c2fd77aace0864790606a5043d1f` |
| Moonlight Qt `vt_metal.mm` | `317802288795c3753e5da847f51a3e19b7ee7ba6` | `2852d4379d5c150a32958ac05d23c3df4c830a65` |
| Moonlight Qt `pacer.cpp` | `317802288795c3753e5da847f51a3e19b7ee7ba6` | `2985df82fb0d82482b54ef799e3928e2fa2988f3` |

[Sunshine](https://github.com/LizardByte/Sunshine/blob/a679b059096bcdd502e2192ce4f6a3c3732fbaae/src/stream.cpp#L1673)
groups packet sends, limits batch sizes and carries rate-control work across
frame boundaries. Its fixed rate assumption is not copied into Sanser.
[Moonlight's Metal renderer](https://github.com/moonlight-stream/moonlight-qt/blob/317802288795c3753e5da847f51a3e19b7ee7ba6/app/streaming/video/ffmpeg-renderers/vt_metal.mm#L912)
uses a newest-frame slot and frame-ready notification for display-link work,
and has a direct-draw path when that display link is not used. Its
[pacer](https://github.com/moonlight-stream/moonlight-qt/blob/317802288795c3753e5da847f51a3e19b7ee7ba6/app/streaming/video/ffmpeg-renderers/pacer/pacer.cpp)
bounds queued frames and wakes workers when there is work.

## Sanser changes

1. **GPU completion cannot lose a pending draw.** A failed nonblocking GPU-slot
   acquisition now records a deferred draw under the renderer lock. Completion
   of the current Metal command schedules one coalesced main-thread draw of
   the newest pending picture. It respects a future presentation deadline and
   does not block the decoder or submit another in-flight GPU command. The
   timed fallback remains for window/status redraws and unavailable drawables.
   This removes a specific reliance on the next periodic callback after a busy
   GPU; it is not evidence that every frame currently waits a whole display tick.
2. **Small timer groups.** Ultra can send ahead by at most approximately 500 us
   while fewer than four datagram attempts have used the current wake group.
   Larger positive pacing debt still waits. The sender's existing absolute
   bandwidth timeline, frame budget and bounded idle credit remain in force.
   Group state carries across frames/repairs, and each datagram still obtains
   the shared socket mutex individually. This is timer grouping, not UDP
   payload concatenation or a change to authenticated packet numbers/MTU.
   Auto keeps its existing per-datagram pacing behavior.
3. **Numeric diagnostics.** Reports retain `udpPacingWaitCalls`,
   `udpPacingGrouped`, `renderBusyDeferrals` and `renderCompletionWakeups`.
   A completion wake counts only when a draw is actually scheduled, including
   coalescing with an already queued draw. No screen/input content is recorded.

## Validation and limits

The production renderer test deliberately occupies its GPU slot, submits the
newest decoded picture, and completes that slot without a periodic draw event.
The completion schedules exactly one draw even if notified repeatedly, and
preserves the newest-frame queue. No real screen is captured.
An idle periodic callback with no fresh picture does not create a completion
redraw loop or another GPU wakeup.

A deterministic 256-datagram trace, with controlled 25 us wait overshoot,
uses the production pacing-group policy and checks the advance/packet bounds,
unchanged bandwidth deadline, cross-frame debt and unchanged Auto behavior:

| Virtual bitrate | Per-packet waits | Grouped waits | Final-send advance |
| --- | ---: | ---: | ---: |
| 20 Mbps | 255 | 127 | 480 us |
| 40 Mbps | 255 | 85 | 0 us |
| 80 Mbps | 255 | 63 | 360 us |

This establishes fewer timer wake calls in that trace, not 1–3 ms of measured
network/game improvement. In particular, the 40 Mbps trace finishes at the
same time despite fewer waits. Hardware scheduling, link capacity, receive
jitter, NIC bursts and real-world loss are not simulated by this test.
The Windows production pacing fixture also reports real runner wait counts
without enforcing a fragile wall-clock performance threshold.

Physical A/B validation should keep route, FPS, resolution and bitrate fixed:
compare decoded-frame age, send/pacing/overshoot, actual loss/repair counters,
completion wakeups and visible smoothness over the same moving scene. Existing
one-second report averages are window summaries, not per-frame P50/P95. A real
frame-distribution or input-to-photon measurement is required before claiming
an end-to-end millisecond saving. No buffer/repair deadline was shortened merely
to improve an overlay number.
