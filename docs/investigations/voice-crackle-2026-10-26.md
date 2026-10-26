# Voice crackle while Mac shares to Windows

The user narrowed the report to microphone speech in group voice: Mac hosts
Share with multiple computers, Windows joins, and shared system sound is clean
until voice/microphone is used. This investigation focuses on voice processing,
not on replacing the system-audio transport or changing video latency settings.

## Reproduced defects

### Reference dropouts changed the microphone sample timeline

SpeexDSP preprocessing uses an overlap/add frame of history. In Sanser's 48 kHz,
240-sample configuration this is 5 ms. The old AEC returned current raw microphone
PCM when render reference was unavailable, but returned delayed processed PCM
when it was present. On the first missing reference after processing, it destroyed
and recreated both the echo canceller and preprocessor. Repeated reference holes
therefore switched sample time and repeatedly discarded the overlap history.

An identical device-free probe was compiled against the actual `v2.2.23` sources
and the repair, using the same SpeexDSP 1.2.1, PCM and timestamps. A 500 Hz near-end
tone was used with silent render blocks present in four of every five blocks, or
one of every five blocks. Only the final three seconds were counted.

| Reference availability | 2.2.23 nearly blank 5 ms blocks / 600 | Repaired |
| --- | ---: | ---: |
| Four of five blocks | 120 | 0 |
| One of five blocks | 120 | 0 |

This reproduces speech chopping without a network, microphone or speaker. It does
not establish the actual render-reference coverage on the user's devices.

The repair keeps preprocessing history advancing and aligns dry microphone PCM
to its same one-block delay. It crossfades aligned wet/dry output, uses dry speech
when there is no meaningful recent playback, retains adaptation through brief
reference holes and resets acoustic adaptation separately after a long gap.
A microphone timestamp discontinuity still clears the pipeline; Mute/Leave destroys
both delayed PCM and adaptation. The processing delay is consistently 5 ms,
including the safe dry path, rather than alternating between zero and 5 ms.

### The mixer distorted an unclipped single speaker

The previous mixer applied `tanh()` to every output sample. This is nonlinear even
for a single speaker whose decoded PCM is within the valid output range. In a
second deterministic probe, 100 voice packets were encoded with the production
Opus encoder and decoded both by the mixer and an independent Opus decoder.

| Measurement against the same decoded PCM | 2.2.23 | Repaired |
| --- | ---: | ---: |
| Fitted mixer gain | 0.88537 | 1 |
| Nonlinear residual after fitting gain | 4.03194% | 0% |
| Decoded packets / concealed packets | 100 / 0 | 100 / 0 |

This is added **mixer** distortion, not the overall codec or physical-call THD.
The repair mixes in float and uses the existing Opus library's stateful
`opus_pcm_soft_clip`: in-range audio is unchanged, while overloaded sums receive
soft clipping. No new codec/library or media protocol is introduced.

## Diagnostics and regression coverage

`SNVOICE_HEALTH` exposes numeric capture levels/clipped-frame counts before and
after AEC, capture/receive queue drops, stale capture, worker wake gaps, decoded/
concealed/late packets, mix overload and output underflow/reprime counts. AEC
reports reference misses, dry blocks and resets. These distinguish clipping,
processing discontinuities and delivery/playout starvation on a real call.
No microphone PCM, encoded speech, device identifiers or speaker identities are
written to the connection report. Consent and forwarding gates remain unchanged.

Regression checks include silent, entirely missing and intermittent references;
active-reference holes with independent near-end speech; a fresh capture clearing
its delayed block; an independent decoder comparison for single-speaker PCM;
overloaded multi-speaker sums; existing 0–100 ms acoustic multipath/double-talk
checks; and privacy filtering for the new numeric metrics.

## REA and primary sources

REA 6.5.0 statically inspected the installed Parsec Mach-O dylib. Imports establish
AudioQueue input/output API use; they do **not** reveal its jitter policy, AEC
algorithm or runtime sound quality. Parsec was not launched or instrumented.
REA also imported the pinned Mumble source snapshot as `historical-reference`,
not as recovered native/binary implementation. Algorithm comparisons below come
from reading that source. No competitor implementation was copied into Sanser.

- Mumble commit `162f42805ce095c37ab57f1d81ba92cdb31bbac5`:
  [AudioOutput.cpp](https://github.com/mumble-voip/mumble/blob/162f42805ce095c37ab57f1d81ba92cdb31bbac5/src/mumble/AudioOutput.cpp)
  mixes in float before conversion;
  [AudioOutputSpeech.cpp](https://github.com/mumble-voip/mumble/blob/162f42805ce095c37ab57f1d81ba92cdb31bbac5/src/mumble/AudioOutputSpeech.cpp)
  has independent speaker jitter/decoder state and fade handling;
  [AudioInput.cpp](https://github.com/mumble-voip/mumble/blob/162f42805ce095c37ab57f1d81ba92cdb31bbac5/src/mumble/AudioInput.cpp)
  separates mic, render reference and acoustic processing.
- [Opus soft clipping API](https://www.opus-codec.org/docs/opus_api-1.5/group__opus__decoder.html):
  state persists between blocks and in-range signals are left unchanged.
- The pinned [SpeexDSP 1.2.1 source archive](https://downloads.xiph.org/releases/speex/speexdsp-1.2.1.tar.gz),
  SHA-256 `8c777343e4a6399569c72abc38a95b24db56882c83dbdb6c6424a5f4aeb54d3d`,
  `libspeexdsp/preprocess.c`: analysis overlap and output overlap/add explain the
  one-frame timeline that dry bypass must preserve.

REA identities, source blob verification, hashes and numeric comparisons are in
[the evidence record](voice-crackle-2026-10-26-evidence.json). Temporary artifacts
and synthetic probe sources are in `/private/tmp/sanser-voice-research-2026-10-26/`.
Unaligned 500 Hz correlation in the raw probe output must not be interpreted as
signal inversion: a 5 ms delay is two and a half periods at that frequency.

These are source fixes subsequent to published 2.2.23. Physical Mac-to-Windows
calls, speakers versus headphones, real reference timing, device clipping and
school-Wi-Fi loss still require listening and numeric-report validation.
