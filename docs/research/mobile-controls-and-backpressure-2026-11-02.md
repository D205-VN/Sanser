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
verifies capture resumes, then offers 8 KiB pictures every 16 ms. It checks that
raw capture admission applies backpressure, every admitted reference is received,
and no encoded video queue is discarded. Genuine fragment loss remains repairable.

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
