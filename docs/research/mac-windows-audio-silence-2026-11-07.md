# Windows system audio becomes inaudible on the Mac viewer

## Live evidence

The user confirmed that the existing Direct Mac-to-Windows session remained
open while Windows video audio became inaudible. Changing the Mac output was
an intentional workaround and reportedly restored sound for about three
seconds. Seven route changes in the report are not evidence that automatic
route selection caused the problem.

During a user-confirmed inaudible interval, the saved connection report showed
approximately 200 audio datagrams/second, successful Opus decoding, increasing
nonzero-signal frame counts and increasing AudioQueue completion counts. There
were no reported authentication, replay, decoder, converter or device-clock
errors in those samples. The Mac default output queried during investigation
was unmuted, at about 19% volume, with a nominal sample rate of 44.1 kHz.
These counters do not prove that samples reached the physical headphones.

There was also a long gap in audio reports, followed by audio reports resuming
on the same session without another output-route recovery. The old receiver
reported only after accepted packets, so that gap cannot distinguish absent
traffic from packets rejected before the reporting block. A thread sample
showed waiting audio workers, rather than a demonstrated Core Audio deadlock.
Local header capture was unavailable without a macOS administrator password;
no network/security settings were changed.

## Local patch

- The legacy Mac output now converts decoded float samples to interleaved
  signed PCM16 before enqueueing to Core Audio, matching the format used by
  the shared macOS player. Network/Opus formats, channel order, input sample
  rate and the 20 ms hardware queue budget are unchanged. Conversion scratch
  storage is reused; nonfinite samples are silenced and full scale saturates.
  This is a targeted output-format trial, not proof that the system converter
  caused this physical failure.
- Audio receiver reports are independent of accepted packets. Timed receives
  retain health reporting during silence and rejection, including numeric
  `audioReceiveIdleMs` and `audioValidIdleMs`. Normal authentication and replay
  rejection remain enforced. The desktop report keeps these fields without
  exporting device names or credentials.
- Windows legacy capture retries device-session exceptions with bounded
  150–2000 ms backoff, including errors outside its previous small HRESULT
  allowlist. Each attempt recreates device discovery. The UDP sender remains
  alive across retries, preserving its authenticated sequence/nonce history.
  This addresses a separate permanent-stop path; it is not established as the
  cause of the live inaudible interval where signal continued arriving.

Microsoft's [GetBuffer documentation](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer)
describes retrying temporary capture-buffer failures and recreating a client
on repeated failure. Apple's local SDK AudioQueue header documents `IsRunning`
as queue state; it is not a physical audibility test.

## Verification and remaining acceptance

Synthetic tests exercise PCM16 byte layout, saturation/nonfinite handling,
route/clock/converter recovery, rejected-packet and idle reporting, clean
receiver shutdown, and repeated capture failures with bounded retry and
transport continuity. The native Mac client builds. The desktop numeric audio
report test passes. Full native GUI checks require AppKit access outside the
sandbox; running them without that access failed during test-view creation.

After the user explicitly disconnected and exited Sanser, the Mac trial was
installed at the existing application location, signature-verified and opened.
The previous app bundle was preserved locally for rollback; account/preferences
were not removed. This is a local trial under version 2.3.4, not a new release.
The Mac output-format trial needs a new viewer process and a real listening
test with continuous Windows audio for at least 60 seconds, first on the
existing output and then across a headphone/speaker switch. Windows device
capture and its full sidecar compilation still require Windows validation.
Do not label the user's three-second silence problem resolved before that
listening test.

## PCM16 listening result and continuous-output trial

The user reported that the first PCM16 trial became inaudible again after a
little over one minute. They also confirmed that Safari/YouTube on the same
Mac output remained audible. The new session still showed about 200 valid
audio packets per second, growing signal/completion counters, and no reported
authentication, converter or output API errors. PCM16 conversion alone did not
resolve the problem and must not be presented as the fix.

The next local trial changes the legacy device-feeding model. Two fixed 10 ms
Core Audio buffers are continuously refilled in the callback, including silence
when network samples are absent. Producers append converted PCM into a fixed
ring, normally at most 20 ms (or one legal larger input packet), with no device
calls in the UDP reader. Callback processing allocates no sample storage and
calls enqueue outside the state mutex. Queue-local gain is checked against the
user-configured gain; this does not modify system volume. The selected/default
physical device UID is explicitly bound when creating the output, while the
existing route/source/rate checks recreate it on device changes.

Unlike a simulated callback's successful return, audibility cannot be inferred
from these changes. They test the hypothesis that device queue starvation or
automatic device tracking contributes to the inaudible output. The previous
physical session did not establish that hypothesis as the root cause.

A three-minute virtual playback test exercises a two-second network gap,
resumption without a new start, bounded queued PCM, callback failure while the
software ring is full, converter failure, device rebinding, and restoration of
an unintended queue-local gain change. Optional output metering and actual
queue gain are now logged numerically; unsupported queries are not errors.
Audio reports also go to the existing private sidecar log when debug logging
is enabled, allowing investigation if this trial also loses sound.

For comparison, [Moonlight's SDL renderer](https://github.com/moonlight-stream/moonlight-qt/blob/master/app/streaming/audio/renderers/sdlaud.cpp)
bounds pending audio and checks output-device state rather than treating a
successful sample submission as sufficient evidence. This comparison informed
the checks; no source from that renderer was copied into this patch.

The continuous-output trial was subsequently installed after the user exited
the app, with private sidecar logging enabled for this local run. Four native
checks (including the full client latency test) and the numeric report test
pass. At over two minutes into the new physical Direct session, the log showed
one output start, no output recovery/API error, queue gain 1.0 and nonzero
output level-meter peaks. Software-ring drops were still observed, so this
does not establish lossless playback. The user then confirmed that sound
remained audible continuously beyond three minutes, past the prior one-minute
failure interval. This is a successful local listening test, not a broad
hardware/OS acceptance claim or a published release.

## Separate live video finding

The user reported picture stutter and Arrival jitter excursions up to about
44 ms while that same audio test remained connected. A subsequent 20-second
window recorded 488 completed video assemblies, 69 dropped assemblies,
235 NACK packet requests, five NACK recoveries and 85 NACK timeouts. Those
counts are different event types and repeated requests are possible: they
must not be divided into a packet-loss percentage. The render intervals
recorded 460 presented frames (about 23 fps), with no render-queue/late drops
and no decoder errors. Their upstream source frame rate was not independently
measured. The rolling jitter summary had P50 14.6 ms, P95 25.3 ms and maximum
46.7 ms at one observation, while Wire RTT P50 was about 11.9 ms.

The jitter statistic is an EWMA of receiver complete-packet interarrival
delta minus sender media-timestamp delta, observed after reassembly and before
video jitter/decode waits. It includes loss/reassembly/repair timing, rather
than being an independent measurement of router delay. This live evidence
supports investigating video delivery/repair and frame cadence before GPU
render tuning; it does not identify Wi-Fi band, an ISP, or any one network
device as the confirmed cause. Numeric samples were retained locally for
follow-up without audio content or account credentials.
