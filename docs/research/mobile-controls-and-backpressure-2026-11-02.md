# Mobile controls and source backpressure

The user still reports severe mobile stutter after the two host-only previews.
The last local Direct host report (Nov 1, 21:47) contains 443 queued repairs and
four sender queue drops in roughly eleven seconds. It does not include the
receiver's decode/display/audio counters, so it cannot identify every remaining
bottleneck or establish that the physical network is responsible.

## Bound work before encoding

For a receiver explicitly advertising a mobile video ceiling, `acceptsCapture`
checks the already queued fresh video's serialization time. Above approximately
two frame intervals (minimum 20 ms), the single-peer host skips superseded **raw
captures**, rather than producing more encoded reference frames and later
clearing their queue. Existing desktop peers retain their behavior. Sharing
groups do not let a single slow viewer apply this global capture gate.

Mac keeps the newest raw image and refreshes it after the queue drains, including
when the desktop becomes idle. Windows gates capture/encode while retaining
control, audio, and keyframe-request processing. This is overload protection,
not a promise of constant 60 FPS on an insufficient link. It can temporarily
reduce captured FPS instead of causing repeated reference loss and IDR requests.

The real UDP mobile pacing regression negotiates 2 Mbps, sends a 128 KiB IDR,
verifies capture resumes, then offers four 8 KiB raw-picture candidates per
producer wake. A burst preserves overload even when CI coalesces the producer's
requested 16 ms sleep to 32 ms. It checks that
raw capture admission applies backpressure, every admitted reference is received,
and no encoded video queue is discarded. Genuine fragment loss remains repairable.

CI then caught an initial-IDR failure on a loaded Mac runner. A deterministic
6 ms delay per original datagram reproduced it locally: zero complete pictures,
both priority keys delivered, one sender queue discard. The fixed desktop
500 ms expiry bound was too close to a 128 KiB IDR's 2 Mbps serialization time.
Explicit mobile IDRs now get a 1.5-second expiry bound, allowing their real
serialization and scheduling overhead; receiver expiry and bounded repair cache
agree. P-picture and
legacy desktop limits remain unchanged. This extends an expiry deadline, **not**
a mandatory presentation buffer; a complete picture is still delivered at once.

A subsequent macOS CI run measured 48 injected 6 ms sleeps taking 1500 ms in
total (about31 ms per datagram), despite the source's requested pacing. The
native send/receive and Opus workers had no explicit QoS. They now request
interactive QoS on Apple at thread entry; iOS session/audio queues also declare
their purpose. This avoids inheriting utility/background scheduling from their
creator. A worker test starts at background QoS, applies the media policy, and
verifies the requested interactive class. Other platforms retain their current
thread policy. Apple documents QoS as affecting scheduling and timer latency:
https://developer.apple.com/library/archive/documentation/Performance/Conceptual/EnergyGuide-iOS/PrioritizeWorkWithQoS.html

The CI measurement is direct evidence of a scheduling problem in the test
environment, not proof that the user's phone has the identical cause. The
loaded-sender regression prints actual wait time to compare subsequent runs;
physical phone/audio tests remain required.

QoS reduced the injected per-packet waits in the next CI run, but the accumulated
injected work still exceeded the explicit 1.5-second expiry. Dropping that IDR
was correct; requiring unlimited accumulation would contradict the latency cap.
The final regression therefore separates a single 300 ms scheduling interruption
(the IDR must complete) from a 1700 ms interruption (the stale IDR must expire,
then a fresh independent picture must resume the stream). These are actual
socket tests; only the injection is synthetic. No further production deadline
increase was made to accommodate arbitrarily slow CI sleeps.

## Phone-specific controls

- Direct touch: tap, drag, long-press right click; two fingers zoom/pan.
- Touchpad: relative cursor, tap, long-press right click, two-finger scrolling.
- Viewport mode: local 1–3x zoom/pan with a reset-to-fit action. Coordinates undo
  zoom, pan, and letterboxing before being sent to the host.
- Keyboard: existing ASCII text entry plus arrows, function keys, and shortcuts.
- Game overlay: WASD stick, Shift, E, Esc, Space. It maps keyboard keys; it does
  not advertise or emulate a native Xbox controller. Unsupported gamepad
  capability remains false.

Input messages carry a session fence. Moves coalesce at 8 ms; reliable button/key
transitions are ordered. Held keys track independent owners so a stick or game
button cannot release another control's key. Cancellation, mode changes,
backgrounding and teardown release input. View-only users emit no input, while
host-side access checks continue to enforce permissions.

Video decoding, media transport and audio playback stay native. Only input,
viewport configuration and numeric diagnostics cross Flutter channels.

## Receiver diagnostics

A one-second native snapshot reports received video packets/completed frames,
NACK requests, lost fragments, reference drops, and Opus decode/PLC/late counters.
The UI derives received FPS using a monotonic local clock; this is **not display
FPS** or an end-to-end latency measurement. Raw RTT remains visible.

Settings can copy the last 120 numeric samples for investigation. The export
excludes session IDs, tokens, endpoints, input contents, compressed media and PCM.
The last samples remain available after disconnect. No report is automatically
uploaded. Physical phone/network smoothness still requires a new device test.
