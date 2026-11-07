# Completed video frames could be re-created by their own repair

## Physical symptom and verified code defect

While the user kept a Mac-to-Windows Direct session running, a 20-second
sample recorded 488 completed video assemblies, 69 dropped assemblies,
235 NACK packet requests, five recoveries and 85 NACK timeouts. The renderer
presented 460 frames, without render-queue/late drops or decoder errors.
These are different event counters, not a measured packet-loss percentage.

The legacy Windows host retransmits an entire encoded packet in chunks,
assigning fresh authenticated fragment sequence numbers. The Mac previously
erased an assembly immediately on completion and remembered no completed
packet IDs. When a repair filled the last missing fragment, its later repair
fragments could create a new partial assembly for that already completed
packet. The new assembly could time out, generate redundant NACK/keyframe
pressure, or complete again and contaminate complete-packet arrival timing.
This is a demonstrated defect; the physical sample alone does not establish
how much of the observed stutter it caused.

## Patch and bounds

After normal metadata validation, HMAC validation, replay checking and
decryption, the reassembler checks a completion window scoped to media epoch.
It remembers 512 packet IDs per epoch and retains at most two epochs. IDs
older than that epoch's window are retired as stale. Previously unseen frames
inside the window may still arrive out of order and complete normally. A new
media generation clears the window so reset packet IDs remain usable.

Already completed fragments neither allocate a new assembly nor reach the
decoder/arrival-jitter input again. Separate numeric counters record completed
duplicates and stale-window fragments. The software does not relax media
authentication or increase render/jitter/repair deadlines in this patch.
Periodic video counters also reach the existing private sidecar log.

The user-confirmed stable continuous audio output code is byte-for-byte
unchanged. The Windows sender and media wire format are unchanged, allowing
this receiver patch to be tested against the installed Windows host.

## Validation

The new production-reassembler regression failed before the patch with
“Late repair fragment recreated an assembly for an already completed video
frame,” then passed after it. It covers the missing-fragment repair followed
by its late tail, repeated full repairs, false NACK suppression, rejection
before completion filtering, stale-ID eviction, unseen reordered frames and
generation reset. Five native checks pass, including the client latency and
stable audio tests; 30 desktop connection-report checks pass, including
numeric retirement counters with credential/device fields excluded.

After the user disconnected and exited Sanser, the patched Mac client was
installed, signature-verified and opened. The installed native binary matches
the tested build; the previous app was preserved locally for rollback. This
is a local trial under version 2.3.4, not a published release.

## Physical retest and selective fragment repair

The user retested the receiver-only build and still observed stutter and a
blur approximately every three seconds. The retained log contains 250
intervals with video: 6,553 complete assemblies, 875 dropped assemblies,
7,949 retransmitted fragments, three retransmit-completed assemblies,
2,787 NACK packet requests, 34 NACK recoveries and 1,056 timeouts. Completed
duplicate filtering retired 531 fragments. These counters overlap and do not
constitute a loss percentage. The receiver change alone did not resolve the
physical symptom. The log does not prove the exact cause of the blur period.

The host currently services whole-packet repair in six-fragment chunks with
a 24-fragment/2,200-microsecond budget per capture iteration. Replaying a
200-fragment frame from fragment zero to recover a hole near its tail can
take many iterations and miss the client's 60-ms Ultra repair deadline.
It also consumes bandwidth sending data the viewer already has.

The follow-up adds optional `missingRanges` to authenticated `video-nack`:
flat triples `[packetId, firstFragment, lastFragmentInclusive, ...]` alongside
the original `packetIds`. The receiver snapshots actual assembly bitmaps,
never advertises a truncated hole list, and limits requests to eight packet
IDs and twelve ranges. The complete authenticated worst-case request is
tested below a 1,200-byte UDP payload. Not-yet-observed tail fragments of an
in-flight frame are excluded until the sender burst stops or a successor
frame arrives. Old hosts ignore the extension and use packet IDs as before.

The Windows host validates the complete hint array, sorts/merges ranges and
sends only the selected fragments through the existing authenticated
packetizer and bounded repair budget. A short send or would-block preserves
the precise repair cursor. Missing/invalid hints and overloaded coalesced
requests retain whole-frame fallback. Epoch checks, cache expiry, the
priority control path and the stable audio output are preserved. No jitter,
render or repair deadlines were raised to disguise the stalls.

Portable tests cover sparse/tail loss in a 200-fragment frame, overlapping
ranges, partial sends, invalid hints and all-or-nothing range caps. Mac
production-reassembler tests cover bitmap hints and authenticated request
size. A Windows production-path test exercises actual control parsing,
cache and encrypted fragment emission, would-block and generation changes;
it must be run on Windows before claiming that host build is validated.

This follow-up requires both the Mac receiver and Windows host to be updated.
It has not yet been physically validated on the user's two machines.
Compare complete/presented cadence, completed-duplicate retirement, repair
timeouts and jitter with the same route/configuration. The source video and
network can vary between runs, so short before/after samples must not be
presented as a controlled bandwidth or latency benchmark.
