# iPhone → Mac Direct video stability, 2026-11-09

## Release CI follow-up

Two macOS CI runs completed the lossless paced IDR but each sent 11 repair
datagrams, failing the existing at-most-four assertion. Their injected sender
pauses measured about 372 ms and 300 ms. The input regressions passed. One
earlier run also failed the broadcast isolation timing assertion; the next run
passed it without a broadcast change. These results must not be described as
an entirely green release check.

A deterministic added case reproduced an obsolete paced-tail estimate: after
consecutive unique fragments resume at 650 ms, the original frame-start estimate
already says the still-paced suffix is missing. It failed before the correction.
For mobile receivers, the tail estimate now also accounts for the most recent
unique fragment plus serialization of the remaining suffix. Duplicate fragments
cannot refresh this estimate. Interior-hole recovery and the original hard
expiry remain bounded; completed pictures are still delivered immediately.
The deterministic recovery tests and real UDP pacing/loss tests passed locally
after this correction. Final CI and physical-device verification are recorded
separately; this does not establish that all perceived stutter is eliminated.

The user reported visible video stutter while audio remained normal on the
personal-device Release build 2.3.6 (20310). The session stayed connected during
inspection; neither screen nor audio content was recorded.

## Evidence from the running Mac host

The local connection report was still updating and marked `ended: false`.

- Direct path probes were approximately 7–9 ms. Probe loss was 0%; this does
  **not** measure video packet loss or prove that media delivery is lossless.
- The receiver advertised a 6,000,000 bps video ceiling; the host's adaptive
  budget had reached 1,000,000 bps.
- The cumulative video queue drop counter reached 12, with repair packets
  continuing to increase. A queue drop is not necessarily one displayed frame.
- Thousands of premature repair requests had been ignored because their
  originals were still in the sender's pacer. That counter is not packet loss.
- Encoder queue work averaged around 0.14 ms and encode-plus-publication around
  11.1 ms. These are cumulative averages, not P95 or display latency.
- The current iPhone build had no saved report in its Documents directory, so
  receive/decode/presentation timing on that device was not directly collected.

## Source defect addressed

`Peer` previously compared newly **encoded/enqueued** payload bytes against
receiver delivery in a single feedback window. A large paced picture could be
counted as offered before it had left the local queue. Window boundary skew
could therefore look like network congestion. Healthy low-rate windows below
32 KB also failed the activity threshold used for bitrate recovery.

For receivers that negotiate the mobile bitrate limit, the patch:

1. Counts original video payload bytes after a successful socket send. Local
   transient send failures and repair duplicates do not inflate that count.
2. Compares delivery across up to four feedback samples, with at least one
   second of evidence before delivery shortfall/loss triggers this policy.
3. Uses aggregate activity for recovery so low-rate video is not stuck solely
   because each half-second sample is below 32 KB.
4. Preserves RTT-based backoff, source capture admission, queue expiry, repair
   eligibility, media authentication, and the desktop-only feedback policy.
5. Adds numeric-only window sent/delivered bytes, duration and shortfall fields
   to the existing transport log and desktop report allowlist.

Audio capture, codec, playback and volume settings were not changed. No iOS
presentation change was made without device timing evidence.

## Validation and remaining work

The six mobile native tests passed, including real UDP pacing/loss repair,
mobile ABI, PCM playback and rate control. New deterministic cases cover paced
boundary skew, low-rate recovery activity, persistent loss, and removal of old
congestion evidence. The Rust report allowlist test passed.

The ARM64 Mac host built successfully. Four selected host tests passed: rate
control, mobile pacing, transport and transient/fatal socket recovery. An initial
pacing run during build activity failed to drain every admitted picture. A
diagnostic rerun and three subsequent comparisons each of the unchanged 2.3.6
transport and patched transport passed. This was not sufficient to identify the
initial intermittent failure's cause; physical-device stability remains required.
The prepared host is under `artifacts/iphone-video-stability-2026-11-09/`.

This removes an identifiable feedback-accounting defect; it does not establish
that all stutter is fixed. The updated Mac host still needs a fresh physical
iPhone session, with the same screen activity, to compare bitrate, queue drops
and perceived smoothness. Public release 2.3.6 and the running process are not
changed by editing these sources.

## Follow-up physical test and iPhone presentation patch

The host patch was installed after the user disconnected and quit Sanser. The
original host, main executable and resource signature were backed up locally;
the installed bundle's signature verified. In the next active iPhone session,
host video queue drops remained zero in the inspected samples, the budget was
around 2.53 Mbps, and path probes around 7.8 ms. This is an observational result,
not an equal-duration benchmark. The user still noticed some stutter. Repair
traffic continued and one control RTT sample reached about 246 ms, so the
remaining issue cannot be assigned exclusively to presentation.

An attempted live Instruments attachment timed out waiting for the device.
There is no CPU profile proving the remaining bottleneck.

The next local iPhone build, 2.3.6 (20311), changes presentation only on iOS:

- A native 60 Hz `CADisplayLink`, in common run-loop modes, consumes the newest
  decoded image. The decoder no longer dispatches per-frame presentation jobs
  to the main queue in this mode.
- A temporarily busy display layer retains the latest image until the next
  tick; newer decoded images replace it without invalidating H.264 references.
- Image format descriptions are reused until dimensions change. Sample times
  use the local host clock rather than a repeated zero presentation timestamp.
- Numeric counters distinguish received, decoded and enqueued-for-display
  pictures, decode queue expiry/overflow, coalesced decoded images, decode work,
  presentation waiting and display backpressure. Enqueue is not scan-out.
- The connection UI shows received and displayed FPS separately. A bounded
  numeric-only local cache (`Library/Caches/sanser-video-health.json`, up to
  120 samples) permits device inspection without recording content or accounts.

The real VideoToolbox test covers manual display polling under backpressure,
newest image brightness, no duplicate image, no per-frame UI callback, no IDR
request from display backpressure, reset fencing, and bounded/private numeric
logging. The controller test verifies displayed FPS and duplicate-snapshot
handling. These tests and Flutter analysis passed; Xcode built and signed an
AOT Release app for the user's iPhone XS Max. This build still needs installation
and physical testing before claiming smoother video. System audio is unchanged.

## Continued session: confirmed decoder overflow

The first cache inspection of build 20311 contained 64 numeric samples. Typical
cumulative mean decode work was approximately 5.8 ms, queue waiting 0.6 ms,
and decoded-image waiting for presentation 8 ms. No display-layer failures or
busy ticks were recorded. These are snapshots of cumulative means, not P95
per-frame timings. Received/decode counters have different sampling instants.

Later in the same session, `iOSDecodeOverflows` reached 48 and host bitrate fell
back to 1 Mbps, with host video queue drops increasing to 8. The old overflow
counter counted rejected pushes, not distinct overload incidents. Decoding a
single frame was fast enough on average, but the six-job admission limit could
not accommodate the transport releasing its repaired reference history in one
burst. The display-layer change alone did not fix this.

A local synthetic AppKit motion window was shown for about 30 seconds and
closed automatically (1806 redraws). This generated continuous screen activity,
but is not an end-to-end 60 FPS certification: capture admission, delivery and
decode still determine how many of those changes reach the phone.

### Local build 20312

- Replace per-frame decode dispatch jobs with one worker draining a bounded
  ordered compressed queue: at most 24 waiting pictures / 8 MiB, matching the
  transport's existing maximum recovered reference history.
- Decoded presentation remains a single newest-image slot. Compressed history
  is not a render buffer and normally drains immediately.
- A 160 ms age limit bounds queued decode recovery. Genuine expiry/overflow
  clears the obsolete history once and rejects dependent pictures until a new
  fitting IDR. Rejected P-pictures do not schedule repeated reset/request jobs.
- Reset/invalidation runs on the decode worker; old-generation output is fenced.
- A fresh fitting IDR can restart immediately; oversize input stays bounded.

The real VideoToolbox regression uses 24 H.264 reference pictures. The original
six-job decoder requested a keyframe and only presented its bootstrap image
(`keyframeRequests=1`, `presented=1`, expected-test exit 2). The updated decoder
decoded all 24 without overflow or a recovery request. Memory-overflow testing
also confirmed one recovery episode despite 100 subsequent P-pictures, followed
by successful decoding of a fresh IDR. Existing latest-image, reset and numeric
privacy tests passed. Physical iPhone validation of 20312 is still required.

## Build 20312 live result: assembly stalls, no decoder overflow

The user reported heavy stutter/freezes after installing 20312. The device cache
contained 40 samples: decode overflow, decode expiry, display failure and display
busy counters all stayed zero; typical cumulative decode work was about 5.3 ms.
There were intervals with 121–421 incoming video datagrams per second and no
complete received frame, while sampled control RTT remained around 7–20 ms.
This proves that those freezes preceded decoding; it does not identify physical
Wi-Fi loss versus local datagram loss by itself. The host again backed down near
1 Mbps. There was no evidence of a new decode-queue overflow regression.

### Local build 20313: repair budget follows fragment progress

Inspection found two independent assembly-repair defects:

1. `Partial::repairAttempts` allowed only three NACKs over an entire picture.
   Early requests for a still-paced tail could consume all three. Even after
   unique originals resumed, a genuinely lost later tail could not be requested
   again, forcing incomplete-IDR expiry/restart.
2. Tail timing was derived from `videoSendBudget`, whose normal expiry budget
   is capped at 500 ms. That shortened estimated serialization of a slow large
   IDR and caused premature requests for originals not yet sent.

The patch renews the no-progress retry allowance only when a new unique fragment
arrives. Duplicate fragments cannot renew it. The existing minimum retry spacing,
initial picture timestamp, cache/memory limits and hard picture expiry remain.
Tail timing uses the uncapped wire serialization estimate; this never extends
the actual hard expiry.

Before the fix, the new deterministic regression failed with
`fresh fragment progress must renew repair for the genuinely lost tail`.
After the fix it passed, together with the slow-tail and duplicate-fragment
guards. An authenticated real UDP test also stalled one original for 600 ms,
dropped the final original of a paced 96 KiB IDR, and recovered the same picture
without a new IDR or an additional queue drop. All four selected host transport
tests and six mobile native tests passed. No audio or iOS presentation changes
were added for 20313. Installation and physical validation remain pending.

## Build 20313 live result and presentation delay follow-up

Build 20313 was installed and launched on the selected iPhone. The cache's
`appBuild` confirmed 20313. Across the 91 inspected snapshots, overflow, decode
expiry, display failure and display busy counters remained zero. No sampled
one-second interval combined zero complete received pictures with ongoing video
datagrams, unlike the earlier freeze evidence. The host reported no video queue
drops in its inspected samples and a budget around 4.3 Mbps. The user reported
some improvement, but still felt delay and minor stutter; this is not a completed
smoothness validation.

Sampled control RTT median was about 8.7 ms. Snapshot cumulative mean decode work
was around 5.8 ms, decode queue waiting around 0.48 ms, and presentation waiting
around 8.1 ms. Median received/decode/enqueued FPS was approximately 33/34/30.
These measurements used the user's changing screen activity, not a fixed 60 FPS
source; idle or variable-rate capture must not be called a renderer failure.

The mandatory display-link polling added in 20311 deliberately waited for a
refresh tick. The measured half-refresh waiting is contrary to the earlier
lowest-latency design in `mobile-video-latency-2026-11-01.md`. Build 20314 restores
immediate coalesced main-thread readiness notifications: a ready display consumes
the newest image immediately, while the display link only retries backpressure.
The mailbox stays single-image; no image is consumed if the layer is busy. The
24-reference compressed queue and repair fixes remain. The VideoToolbox test
confirms a blocked main loop receives only one readiness notification and the
pending newest image survives backpressure; the other video regressions pass.
Actual presentation-delay reduction still requires a physical 20314 session.
