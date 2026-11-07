# Voice/performance evidence — phase 5a

Source increment, 2026-11-06, version 2.3.3 unchanged. This prepares measurement
of the current native voice path. It does not replace voice with LiveKit, open
microphones, deploy a server or certify an audible improvement.

## In the app

Settings → Diagnostics → **Measured native performance** now has an **Audio and
voice counters** section, separately for hosting and viewing. System sound and
group conversation have distinct counters. Voice PCM underflows, missing-packet
concealment, capture queue drops and near-full-scale microphone frames produce
diagnostic hints only when a recent, valid interval actually increased.

An initial nonzero lifetime count is a baseline, not a new error. Counter or
voice-configuration changes, and gaps over five seconds, establish a new
baseline. Older engines without a voice epoch remain supported; resets whose
counts decrease can be detected, but an unannounced reset that already exceeds
the preceding count cannot be identified reliably.

Counts, observed increases and covered seconds are distinct from timing in ms.
Values older than five seconds or from an ended process are stale. Missing
audio telemetry is unknown, including on mobile, rather than a confirmation
that sound works. No voice support or audio export was added to the mobile UI.

`SNVOICE_HEALTH` retains its compatibility fields and adds `voiceHealthEpoch`
and `voiceWorkerGapWindowMaxMs`. The latter resets after each health report;
the old `voiceWorkerGapMaxMs` is a lifetime maximum and is not used as a rolling
wake-gap percentile. This is worker scheduling, not microphone-to-speaker or
network latency. Counter summaries are bounded to a fixed allowlist per native
report and reuse existing retention/export limits. No PCM, Opus payloads, speech,
device identities or credentials enter the added summary.

## Run a controlled comparison

1. Use the same physical host/viewer, route, media settings, output device and
   workload in both runs. Keep the number of participants/open microphones fixed.
   Run each case for at least 60–90 seconds; startup is part of this observation,
   except for the first counter sample. The tool does not trim a warmup interval.
2. Record conditions explicitly. Test headphones first, then speakers as a
   separate case; also test shared music, two/three simultaneous talkers and
   plugging/unplugging audio devices. A device-switch case must use the same
   sequence in both runs. Never enable a microphone without its user's action.
3. Export diagnostics for each run. Copy
   [the example manifest](benchmarks/voice-ab-manifest.example.json), point its
   two `file` entries to the exports and confirm only conditions actually met.
   If an export contains multiple matching reports, specify `sessionId` and
   `role` in each arm, or supply an explicit single-report file. Ambiguity fails.
4. Run from the repository root:

   ```sh
   node scripts/performance-ab.mjs --manifest /path/to/ab.json --output /path/to/ab-result.json
   ```

The output is created exclusively and does not overwrite an input or existing
output. No application/device is launched and nothing is uploaded.

The comparator refuses numeric deltas when direction, role, transport, protocol,
mode or requested width/height/FPS/bitrate/codec differ or are unknown. Codec
`auto` is unresolved and cannot establish matching codec conditions. It checks
the recorded local OS; peer OS and physical conditions remain operator assertions.
Two identical `direct-lan` labels do not prove identical physical route endpoints.
Each run remains separate: no pooled percentiles or invented missing zeros.

Timing comparisons require at least ten compatible periodic observations per
stage per run. They are not per-frame/screen-to-screen percentiles. Audio rates
require at least 30 **covered** seconds per counter in both runs. First samples,
resets and long gaps are excluded; output shows each counter's coverage and
reset count. Counts have different denominators: frames, packets, blocks or
underflow calls. Their rates must not be summed into one quality score.

A negative change is a numeric change, not a pass result. FEC attempts do not
confirm packet recovery; PLC is not a measured network-loss percentage; AEC dry
blocks alone do not prove echo. A counter decrease does not prove a fix if the
microphone stopped capturing or the participant stopped talking. Always check
captured/decoded/sent activity and listen to the real output.

The existing `performance-matrix.mjs` now carries the same allowlisted audio
counter summaries. Old exports have unknown audio evidence; raw retained counts
are not silently converted to complete-session error rates.

## Verification and next work

- Diagnostics Rust tests cover source/field allowlists, whole counts, bounded
  state, clock/order/epoch/gap baselines, fresh findings and separation from RTT.
- Desktop report tests cover parsing, serialization, export/persistence and
  secret exclusion. UI tests cover voice-only evidence, baseline vs zero, stale
  system/voice counters and ended-process findings; type/lint checks pass.
- Offline matrix/A-B tests cover mismatched conditions, insufficient evidence,
  unknown/zero counts, privacy and CLI input/output protection.
- The Mac native client builds; synthetic Opus/AEC and encrypted three-viewer
  tests pass without opening audio hardware. Windows voice-runtime source
  syntax-checks; this is not Windows runtime or acoustic validation.

Phase 5b now has an [isolated browser Voice Lab](../apps/voice-lab/README.md)
with explicit endpoint/consent/token lifecycle and a bounded WebRTC stats export.
Actual server/device trials, native integration and cross-provider A/B adaptation
remain. This tool currently compares Sanser native reports only. It does
not claim a LiveKit comparison, solve the reported hardware distortion or
demonstrate a reduction in end-to-end milliseconds. No release was made.
