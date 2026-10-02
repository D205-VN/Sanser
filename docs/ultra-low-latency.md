# Ultra Low Latency

Choose **Settings → Stream → Ultra Low Latency**, then reconnect. The preset uses
Auto resolution (up to 1920×1200 on the legacy route), 60 FPS and 25 Mb/s.
Its session/API name remains `competitive` so existing
servers can carry the preference to both native engines. This is an opt-in mode
for stable, low-latency networks; it does not force Direct or bypass authentication.

## Mac controlling Windows

The existing production route uses authenticated, encrypted SNU2 video and
multiplexed UDP input. Tauri starts the engines; screen frames and input do not
pass through WebView commands. This change does not replace that wire format
with SNV2 or remove its relative mouse/gamepad features.

- The Mac renderer retains at most one decoded frame waiting for presentation.
  Newer decode submissions replace older ones; a late callback cannot replace a
  newer submission, including across a sender sequence restart.
- Base render pacing is zero. The policy caps additional adaptive delay at 4 ms;
  Ultra currently does not increase it in response to intentional frame dropping.
- Frame arrival schedules a coalesced main-thread draw. Metal permits one command
  buffer in flight, retains the pixel buffer until completion, and disables extra
  display-sync pacing in this mode. OS scheduling and display scan-out still apply.
- A missing sequence gets a 12–50 ms adaptive reorder window based on arrival
  jitter and control RTT (18 ms at 10 ms RTT with low jitter). Contiguous frames
  have no artificial jitter hold. Intact encoded frames are not dropped merely
  because arrival is late; the newest decoded image replaces old render work.
  Only after a missing sequence expires do dependent frames wait for a keyframe.
- A partial frame missing fragments after its last fragment or a newer frame
  triggers NACK after a 3 ms reordering grace. The UDP loop services recovery
  every 3 ms even during receive silence. OS scheduling can extend these targets.
  NACK retries are 20 ms apart and expire after 60 ms in Ultra; keyframe requests are rate limited to 100 ms.
  Incomplete fragment assemblies have an 80 ms bound to accommodate large IDRs.
- Windows retains retransmission eligibility for 150 ms, independent of the
  receiver deadline, within the existing 32-packet/16 MiB cache bound. Ultra's
  software pacing budget is 3–5 ms per packet with a bounded burst allowance;
  crypto, socket work and scheduling can make actual send time longer. This is
  not a guaranteed network delivery time or a new bandwidth estimate.
- Input retries poll every 2 ms with a 12 ms retry interval. Exhausted retries
  clear queued input and send an authenticated reset to avoid a stuck key.
  Connected gamepad snapshots coalesce per controller and refresh every 50 ms;
  disconnect events retain acknowledged delivery. Relative mouse deltas are
  accumulated, not discarded as if they were absolute coordinates.
- Normal Ultra input batches are capped at four events (previously 24). The
  overlay separately reports host processing time from input ACK RTT. These
  timings are not synchronized samples of the control-ping RTT.
- Desktop Windows hosting omits the added cursor from video; the local client
  cursor remains visible. The standalone host can retain its default cursor
  composition unless launched with `--no-stream-cursor`.
- Windows attempts its existing DXGI/D3D11/NV12 hardware path. If it falls back,
  the Mac overlay explicitly warns **CPU FALLBACK**. It does not silently label a
  CPU session as a working GPU pipeline.

## Native SNV2 transport

SNV2 already carries input over encrypted/authenticated UDP. A dedicated sender
now services bounded control, reliable input, audio and video lanes per datagram.
Video pacing waits are interruptible by input/control. Absolute moves use a
separate newest-state slot; keys/buttons remain ordered. The audio lane is reserved
in the scheduler; this does not add audio/gamepad support to SNV2 platform backends.

Peers negotiate selective ACK support using the keepalive capability payload.
The existing cumulative ACK remains compatible with older peers. Selectively
received input is not retransmitted, but retained until the contiguous ACK advances.
Ultra retries gaps every 12 ms and closes/reset the session if reliable input
cannot be delivered within 120 ms. It never skips a missing key transition and
continues with later transitions. Video queue age is bounded to 40 ms in Ultra
(100 ms otherwise); dropping encoded reference data requests a new keyframe.
Pacing allows 20% framing overhead above the requested encoded-video bitrate.

SNV2 Windows hosting in Ultra requires the GPU path and fails with an explicit
error if unavailable. The Mac SNV2 presenter also limits GPU work to one submission.

## Measurements

The Mac→Windows native overlay starts visible in Ultra. **Control+Option+F8**
toggles it; plain F8 remains available to the remote app. It displays:

| Field | Meaning |
| --- | --- |
| Capture / Encode | Host-local average processing durations, returned in authenticated control pongs |
| Send / pacing | Host time submitting video, including pacing; not a separate queue-delay estimate |
| Network RTT | Client-local control ping/pong duration |
| Arrival jitter | Smoothed variation in completed packet arrival spacing |
| Decode | Local time from VideoToolbox submission to output callback |
| Frame queue | Average decoded-frame wait before a draw consumes it |
| Render GPU | Metal command GPU execution time; excludes scan-out |
| Input ACK RTT | Client-local interval from input batch submission to host ACK |
| Host input | Host-local time applying and processing the acknowledged input batch |

Unavailable or stale samples show `—`. Red values indicate per-stage budgets,
not measured total end-to-end latency. CPU fallback keeps the warning visible.
The bounded local diagnostics report also retains frame-queue and GPU render
measurements. No screen content, typed text or session credentials are added.

## Validation and limits

Native regression tests cover reordered video within the deadline, loss after the
deadline, late repair rejection, IDR recovery, sender generation reset, pacing
bounds, priority lanes, duplicate input, selective retransmission and reliable
input expiry. Loopback and codec tests check interoperability with the actual engines.

These checks do not establish physical Mac–Windows latency. Measure Direct and
Relay separately on real devices. Moving a signaling server to a Mac or using
a Cloudflare tunnel does not itself guarantee 5–15 ms input-to-display latency.
