# System audio: Windows controlling a Mac

The cross-platform SNV2 viewer previously decoded video and injected input but
had no Windows audio output. Both launch paths also explicitly disabled SNV2
audio and runtime capabilities always reported audio unavailable. Changing the
Windows speaker volume could not fix that missing implementation.

The new path supports **one Windows viewer connecting to a Mac host running
macOS 13 or later**. Both computers need a build containing this change. On the
Mac enable **Host → System audio**; on Windows enable **Settings → Audio →
Session audio**. The Mac must grant Sanser Screen & System Audio Recording.
Reconnect after changing session settings. Microphone capture is not enabled.

## Native path and compatibility

- ScreenCaptureKit captures system audio at 48 kHz, stereo, on a separate audio
  dispatch queue. Both planar and interleaved float buffers are validated and
  converted into complete 5 ms PCM16 little-endian blocks.
- The viewer advertises `SAP1` in an authenticated optional keepalive. The host
  sends audio only after this opt-in; muted and existing viewers receive none.
- `SPA1` audio payloads use the existing authenticated SNV2 Audio packet type
  and UDP socket. Each block fits below a 1200-byte datagram, needs no fragment
  assembly and is never retransmitted.
- Control and input remain ahead of audio in the sender. Audio precedes video,
  retains at most eight queued blocks, and discards blocks older than 40 ms.
  The limit is not a mandatory 40 ms playout delay.
- The Windows UDP receiver only validates and queues PCM. A dedicated WASAPI
  worker owns COM/device initialization, rendering and teardown. It targets
  5–10 ms of device padding, supplies silence on underflow, retries missing or
  unplugged devices, and follows changes to the default multimedia output.
  An audio-device error does not close the video/input session.
- `snv2AudioImplementation` is separate from legacy `audioImplementation`.
  Capability detection does not falsely enable other engines or older Macs.
  Both host and viewer honor the persisted Session audio setting, independently.

This first implementation uses approximately **1.54 Mbps of PCM payload**, plus
encryption and UDP framing overhead. It does not add Opus compression, clock
drift correction, a full adaptive audio jitter buffer, group-sharing audio,
Windows-host SNV2 capture, or Mac-client SNV2 playback. Existing Windows-host →
Mac-client legacy audio is unchanged. It is not microphone forwarding or
two-way voice chat. Public WSS fallback retains its existing TCP limitations.

## Verification

Automated coverage includes signed PCM/endian round trips, a sealed datagram
size below 1200 bytes, float clipping and callback boundaries, authenticated
UDP delivery, mute on either endpoint, legacy viewer compatibility, stale audio
discard and input/control priority. A CoreMedia test constructs actual planar
and interleaved audio sample buffers and checks the captured PCM channels.

Windows CI must compile the WASAPI implementation; those tests do not prove
physical speaker output. On two real devices, play non-DRM music on the Mac,
connect from Windows, then test each mute switch, USB/Bluetooth output changes,
disconnect/reconnect, Direct and Relay. Confirm video/input continue when the
Windows output device is unplugged. Do not infer end-to-end audio latency from
the configured buffer limits.
