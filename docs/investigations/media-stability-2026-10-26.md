# Legacy video repair and system audio route recovery

Scope: ordinary Mac viewer controlling a Windows host. This investigation is
separate from the preceding group voice AEC/mixer correction. No release or
installed application is changed by these source edits.

## Evidence and limits

The supplied overlays show healthy capture/encode/decode/render work, with
intermittent control/socket RTT, raw UDP echo and arrival jitter spikes. The
individual probe values are asynchronous userspace observations, not a
synchronized measurement of both network directions. They do not establish
that a particular router, NIC, codec or application queue caused every spike.

Recent locally retained 2.2.23 reports also contain intervals with substantial
Opus retirement followed by concealment. One retained interval spans about
5.0 seconds and adds 347 `opusDropped` and 347 `opusPlc`, with no new output
errors. Other retained intervals have virtually no drops. This is evidence
of intermittent recovery pressure, not proof that every lost-audio episode
has the same cause. Only numeric counts were inspected; no speech or packet
payloads were recorded.

There is no intentional forty-second listening limit in this path. The
existing audio freshness deadline is **40 milliseconds**, not seconds.

## REA and reference review

REA 6.5.0 imported the following exact public source files as
`historical-reference` evidence. Their Git blob hashes were verified against
the GitHub fetch results. This is source review, not a claim to have recovered
a competitor's private implementation or measured its runtime quality.

| Reference | Commit | Git blob | SHA-256 |
| --- | --- | --- | --- |
| Sunshine `src/platform/windows/audio.cpp` | `d0b4a88ef727f8dabf80cc970a14739b55c885de` | `d12d015c0d3a7a140a6f447162a8cccfae48ff40` | `59f11b71816f91f2176a062c26e065638e1c04d5dbcae908835125c94f197e6e` |
| Moonlight `src/RtpVideoQueue.c` | `41cedb89d6b8592f99c09975c764ca3875034726` | `a4e022ee6f4eff1f411fe3ea8512a635c9d4fd75` | `4399b84d71dae1c9e9133a4f23badec065acde569479c3be40db263498dfa32d` |

[Sunshine](https://github.com/LizardByte/Sunshine/blob/d0b4a88ef727f8dabf80cc970a14739b55c885de/src/platform/windows/audio.cpp)
observes default rendering endpoint changes and reinitializes capture when
following the default. Its capture path also distinguishes device invalidation
from ordinary capture timeouts. Sanser previously handled invalidation but
could remain on a valid old default endpoint after an output switch.

[Moonlight](https://github.com/moonlight-stream/moonlight-common-c/blob/41cedb89d6b8592f99c09975c764ca3875034726/src/RtpVideoQueue.c)
keeps recovery tied to frame/FEC boundaries and rejects retired data. Its
packet-loss recovery is not a drop-in replacement for Sanser's authenticated
SNU1/SNU2 packet repair protocol. We preserve Sanser's bounded repair, frame
dependencies and newest decoded-frame presentation instead of copying its
wire protocol or retaining arbitrary old frames.

Microsoft's [device invalidation guidance](https://learn.microsoft.com/en-us/windows/win32/coreaudio/recovering-from-an-invalid-device-error)
and [default endpoint routing guidance](https://learn.microsoft.com/en-us/windows/win32/coreaudio/getting-the-default-device-endpoint-for-stream-routing)
support reopening the current default rendering endpoint after a route change.
No competitor source was added to production.

## Changes

1. **Early video repair.** A hole below the highest received fragment can
   request NACK after a 3 ms reorder grace, even before the last fragment or
   another frame arrives. A partial contiguous prefix with no further fragment
   for 6 ms starts the same grace for tail loss. An arriving reordered fragment
   cancels the interior-hole timer. Existing NACK deduplication, bounded retry,
   assembly expiry, keyframe recovery and render limits remain in force.
   This reduces detection delay; actual repair still needs a network round trip
   and a sender cache hit. It cannot promise zero drops during 200–400 ms spikes.
2. **Retired Opus audio is not missing audio.** After dropping expired PCM
   source packets, reset the decode/playout timeline to the fresh head instead
   of creating PLC for deliberately retired sequence numbers. Previously, that
   PLC could consume time while more fresh packets aged into the same discard
   deadline. Real short sequence loss still uses bounded PLC.
3. **Windows loopback follows the output.** Check the selected default endpoint
   at most every 250 ms on the capture worker and restart when it changes.
   A temporarily absent default endpoint now retries rather than permanently
   leaving the capture loop. Restart clears partial PCM/encoder history and
   applies a short continuity ramp. Transport/authenticated sequences remain
   monotonic across a device restart.
   Transient UDP send failures now retire the affected packet instead of
   terminating the audio capture worker; the next fresh packet can continue.
4. **Mac output follows device/source/rate changes.** Check the selected output
   every 250 ms during active PCM submission. Recreate AudioQueue after either
   a different device or a same-device jack/source/rate change, including an
   old queue that still reports running. Device creation/allocation failures
   enter the existing bounded retry path. Explicit device selection remains
   supported, including fallback when that device disappears.
   The first PCM after a device restart ramps in over 1 ms to avoid a step
   from silence to a nonzero sample on the new output.
5. **Numeric diagnostics.** `opusFreshnessResets`, `outputRouteChanges` and `audioSendDrops`
   distinguish these recoveries from packet loss or playback API failures.
   Connection reports allow only those numeric values, not device identifiers.

## Validation

- Production Mac host/client and all native test targets compile in an isolated
  temporary build; the staged sidecar in the workspace was preserved.
- Thirty native tests passed with synthetic media and localhost transport.
  The existing client latency test also passed separately with AppKit/Metal
  initialization permitted. The new media-only test avoids that requirement.
- Forty seconds of virtual-clock AudioQueue playback covers source changes on
  the same device and switching device/sample rate. No speaker was opened.
- Production parser/reassembler test requests interior repair before receiving
  the frame tail, retains the partial frame and verifies all ten output bytes.
- The new freshness regression fails against `v2.2.23`'s `opus_audio.cpp` and
  passes with the corrected implementation. Normal reorder, real-loss PLC,
  fresh generation handling and resampling tests still pass.
- All 27 Rust connection-report tests pass, including numeric-only privacy
  filtering for the new counters.

Windows compilation, authenticated wire/reset tests and full CI are tracked
on the same development branch. Physical Windows/Mac headphone hot-plug,
long-running listening and real Wi-Fi frame-drop P50/P95 still require a
two-machine test of the resulting installer. A forty-second synthetic test
is not a claim that the user's physical endpoint fault has been reproduced.

Local reference and test evidence: `/private/tmp/sanser-stability-research-2026-10-26/`.
