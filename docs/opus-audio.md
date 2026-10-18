# Opus + PLC audio (development after 2.2.15)

Ordinary **Mac controlling Windows** (Legacy audio) and **Windows controlling
Mac** (SNV2 audio) now negotiate Opus when both engines support it. This source
change is not part of the already published 2.2.15 installers. Group sharing is
still video-only. This does not add mobile audio, microphone forwarding, FEC,
or an audio receiver to platform pairs that previously lacked one.

## Shared codec and receive policy

- Statically link libopus **1.6.1**, with an official tarball SHA-256 pinned in
  `native/common/opus.cmake`. No separate runtime installation, neural models,
  telemetry service or dynamic codec download. First native configuration needs
  access to the source tarball; an offline build may supply CMake's
  `FETCHCONTENT_SOURCE_DIR_SANSER_OPUS` with the extracted verified sources.
- Stereo 48 kHz, 240 samples/channel (**5 ms**), **96 kbit/s CBR**, restricted
  low-delay application. Decoder accepts Opus's legal mono coding of a stereo
  stream (e.g. silence) and still outputs two channels.
- Audio has an independent sequence and a 48 kHz sample timestamp. Control
  packets on a shared socket do not count as lost audio.
- A separate receive worker reorders by audio sequence with a **10 ms** startup
  reserve. Missing blocks at playout deadline call the actual Opus decoder's
  PLC path, rather than substituting a zero-filled packet.
- At most **six consecutive blocks / 30 ms** of PLC. Then stop generating
  concealment and re-prime on fresh audio. Drop packets older than **40 ms**;
  bound queued encoded packets to 12 and retain three on overflow. An OS stall
  cannot build up seconds of audio. An already played/concealed sequence cannot
  be replayed after the stream becomes idle.
- Generation reset clears decoder/sequence/queued data and rejects the retired
  epoch. Reset serializes with the bounded PCM enqueue callback. Codec work
  does not run on the UDP reader, video decoder or device callback.
- Source float stereo at 44.1/48/96 kHz (and validated rates 8–384 kHz) is adapted
  to fixed 48 kHz blocks, preserving partial blocks and fractional resampling
  position across callbacks. This initial converter uses linear interpolation;
  it is not a claim of a high-order anti-aliasing resampler for every source.

96 kbit/s describes compressed Opus payload, not Ethernet/socket throughput.
Wire headers/authentication add overhead. The Opus queue and existing device
queue contribute audio latency independently of video/input latency. These are
bounded budgets, not a guarantee of 5 ms end-to-end sound.

## Negotiation and wire format

**SNV2:** a receiving client advertises authenticated keepalive `SAO1` in
addition to existing PCM capability `SAP1`. A host without Opus support ignores
`SAO1`. A new host keeps PCM `SPA1` for a client that never advertises `SAO1`.
Opus uses the existing authenticated Audio packet/lane with body:

```
SOA1 | audio sequence u64 BE | sample timestamp u64 BE | Opus payload
```

Sequence starts at 1; timestamp is `(sequence-1)*240`. Packet and timestamp
validation precede decode. Transport generation/authentication/replay checks
remain in force. Once Opus arrives, late PCM from before the switch is ignored
until generation reset. The Windows output PCM queue is cleared on that switch
and on generation reset.

**Legacy Windows host → Mac client:** `control-hello` advertises
`audioOpus:true`. Only hosts observing this opt-in send `SNA5` (plain/local
legacy mode) or `SNA6` (existing authenticated encrypted mode). The existing
audio header carries 48 kHz/stereo/240 frames and the same audio sequence;
payload is `SOA1` above. `SNA6` retains ChaCha20, epoch/authSeq and HMAC under
the existing audio key, not a new unprotected media channel. Authenticated
sessions reject plaintext `SNA5`, just as they reject plaintext PCM.

Old client → new host retains SNA1–4 PCM. New client → old host continues to
decode those formats. Opus gets its own reorder/PLC worker but reuses the Mac
A/V synchronization and bounded device submission; it does not add another
24 ms jitter reserve. Retired rekey-grace Opus is not fed into a new decoder.

## Diagnostics and acceptance

`SNV2_AUDIO_HEALTH` and Legacy audio stats expose numeric counters:
`opusDecoded`, `opusPlc`, `opusLate`, `opusReordered`, `opusDropped`,
`opusErrors`; SNV2 also reports `opusTxPackets` and `opusTxBytes`.
No PCM samples, voice content, key material or credentials are added to local
connection reports. Legacy sender bandwidth counts sent datagram bytes rather
than pretending compressed audio is still PCM payload.

Automated checks cover real Opus encode/decode, payload budget, silence,
malformed/timestamp mismatch, reorder/duplicate/late packets, missing-block
PLC with nonzero synthesized signal, finite PLC, stale age, epoch reset and
streaming 44.1/48/96 kHz callback boundaries. Native UDP tests cover Opus opt-in,
PCM fallback both directions, mute and absent audio callback. A Windows-only
synthetic socket test exercises production Legacy sending, authenticated
SNA6/encryption, two epochs and SNA4 fallback without capture or OS input.

Remaining physical checks: both ordinary control directions on Direct/Relay,
speech/music for ten minutes, small/random and burst loss, changing speaker or
USB/Bluetooth output, capture pause/resume and audio/video synchronization.
PLC conceals short losses; it cannot reconstruct arbitrary long missing audio
or fix clipping/feedback/device faults. No claim is made that the user's actual
crackle or every approximately 200 ms network route is resolved.

Opus's upstream license is included under `apps/desktop/src-tauri/licenses/opus.txt`,
bundled as a desktop resource and copied into Windows portable packaging.
