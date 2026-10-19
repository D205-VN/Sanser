# Group system audio — 2.2.16

Mac and Windows hosts share system sound with up to three view-only computers.
Update both host and viewers to 2.2.16. The existing account, invitation consent,
per-viewer authenticated session and input denial remain unchanged. No server
migration, extra port, microphone capture or separate codec installation is needed.

On the host enable system audio in Settings before starting the sharing engine.
Capture settings remain fixed while that group engine is running; restart sharing
after changing the host audio preference. On each viewer, Active session offers
**Mute sound / Unmute sound**. This changes only the current viewer, survives page
navigation/reconnection and never stops video or other viewers' sound. A local
sound preference initially off starts that viewer muted rather than disabling
its ability to unmute later. Unsupported/old hosts may still send video only.

For the separate opt-in microphone conversation feature in development, see
[group voice](group-voice.md). Ordinary system sharing never opens a microphone.

## Native pipeline

- ScreenCaptureKit captures screen and 48 kHz stereo system audio on macOS 13+.
  macOS 12.3 can share video but does not advertise system audio capture. Existing
  Screen & System Audio Recording permission is still required.
- Windows SNV2 hosts use one WASAPI default-render-device loopback worker,
  including group hosts. Convert mono/stereo or the front L/R channels of a
  multichannel mix to stereo, resample to 48 kHz with the shared packetizer and
  emit fixed 5 ms blocks. Recognize float32 and PCM16/24/32, sanitize non-finite
  samples, handle silence/discontinuities, and retry device changes/failures
  without stopping video. This is not a surround downmix or microphone source.
- `Broadcaster::audio` encodes each source block **once** with the shared 96 kbps
  low-delay Opus encoder. Only if a negotiated PCM-only viewer exists, prepare
  one additional PCM serialization, not another Opus encoder. Each Peer encrypts
  independently with its own keys, generation/replay sequence, bounded audio lane
  and socket. PCM fallback costs more bandwidth on that viewer's link.
- Audio uses the priority audio lane and a 40 ms freshness deadline, not the video
  delivery/retransmit queue. Pausing one slow viewer's video does not stop sound
  to others. Existing feedback/congestion policies still apply independently.
- Native Mac SNV2 viewers use a bounded software PCM queue and two 10 ms
  AudioQueue device buffers. Windows viewers retain event-driven WASAPI playback.
  Both use the shared Opus reorder/PLC worker and discontinuity smoothing.

## Independent mute

`--mute-audio` initializes mute before client startup. The session-scoped Tauri
command writes a bounded `audio-muted` instruction to that viewer's native stdin.
It rejects a closed or different session; it never replaces the host engine.
The client immediately gates audio/PLC callbacks, clears pending decoder and PCM
playout, and advertises authenticated `SAM1 + muted` keepalives. The host tracks
preference sequence numbers so reordered older mute records cannot override a
newer preference. Repeated capability/mute announcements repair a lost UDP toggle.
The local gate does not depend on the host acknowledging mute. A tiny already
submitted device tail may remain; no new audio is queued while muted.

Unmute resets pending playout and resumes in the same transport generation;
it does not replay the muted interval. The Opus sequence belongs to the shared
source, so joining viewers can start at any current sequence. Audio is sent only
after that viewer explicitly opts in. Old PCM-only viewers retain negotiated
fallback; viewers without an audio callback receive no audio. View-only clients
still cannot inject keyboard/mouse input at the host.

## Validation and limits

The native group UDP test connects three separately authenticated viewers (two
Opus, one PCM), verifies one encode per source block, exact PCM fallback, live
mute/unmute without losing video, and continued audio after another viewer closes.
Pure conversion tests cover mono, float/NaN, PCM16/24/32, silent and truncated
device buffers. Frontend tests cover session-local mute, native-command failure,
group audio opt-in and input restrictions. No fixture captures speakers or plays
audio on the developer's physical devices.

Still required: physical Windows/Mac combinations over LAN/WAN/Relay, speech/music
listening, USB/Bluetooth changes, OS permission denial, host pause/sleep, burst
loss, three real viewers and A/V synchronization. Synthetic tests do not establish
that every reported freeze/crackle is fixed. FEC protects video; Opus PLC masks
short audio losses but not long outages. Audio does not fix a slow network route.
