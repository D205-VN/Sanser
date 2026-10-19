# Group voice research for screen sharing

Research date: 2026-10-19. Scope: microphone conversation between a sharing host and up to three viewers on Windows/macOS. This is separate from the existing shared computer/system audio stream.

## Evidence boundary

The investigation used REA 6.5.0 on a real published JavaScript voice package and normal source review for open-source implementations. It did not launch Discord/Mumble, use a microphone, join a call, capture live traffic, install software, or decompile proprietary Discord components. These findings do not describe every voice app or prove physical latency/audio quality on the user's machines.

### Published artifact inspected with REA

- Package: [`@discordjs/voice` 0.19.2](https://www.npmjs.com/package/@discordjs/voice/v/0.19.2), a Discord voice integration library, **not the Discord desktop client**.
- Acquisition: `https://registry.npmjs.org/@discordjs/voice/-/voice-0.19.2.tgz`; downloaded bytes were checked against registry SHA-512 integrity before extraction.
- Archive SHA-256: `d73df40425aad46ab0b13e9424731b2855c1c8c52d36b7cf0d2c8cf9a7bf0200`.
- Extracted artifact identity: `ded4ed2501dde89e7504ccf3798cf32aa288717257170fa832b1fa6cdf685686`.
- REA Evidence ID: `ev_88722530275f0c78caef7112ec7c2c3cd6bcb33a8e6421b1a048d8983adb7654`.
- Command: `rea analyze-javascript-application <extracted-package> --source-path dist/index.js --format json`.
- Coverage: one selected shipped JavaScript bundle, 114,617 text bytes and 13,072 visited AST nodes, no parse failures; six inventoried files were outside selected analysis. Static syntax and bindings do not establish runtime behavior.
- Local evidence, downloaded files and selected upstream sources remain in `/private/tmp/sanser-group-voice-research/`; no third-party application/package is added to Sanser dependencies.
- Initial analysis could not compile REA's process-ownership helper because the sandbox denied its compiler cache. The same static operation completed after a scoped escalation; REA/client configuration was not changed.

Observed in the shipped `dist/index.js`, corroborated by the static graph:

| Location | Observation | Relevance |
| --- | --- | --- |
| 120–126, 596–677 | A 20 ms audio scheduler and explicit silence frames | Packetization cadence belongs to voice, not video presentation |
| 999–1107 | Dedicated UDP transport and keepalive lifecycle | Voice traffic should not wait for video encoding/rendering |
| 1713–1817 | Sequence/timestamp progression, RTP packet construction and authenticated encryption | Use explicit stream ordering and existing authenticated transport |
| 1885–1949, 2011–2129 | SSRC-to-user mapping and per-user receive subscriptions | Preserve speaker identity rather than combining unrelated codec states |
| 2795–2801 | Opus transform configuration at 48 kHz, stereo, 960 samples | Discord's wire format is an interoperability choice; Sanser need not copy stereo for microphone speech |

REA graph anchor for `VoiceUDPSocket`: `jsrg_node_099e06007d4aa2654290b61732fab99733630c37c557b725cb03bae2e7174427`. The `dist/index.js` content digest is `a0e4c74c2e2992a0db38823bc495f95a6aacfd88e7d356a743d5889cf06442ec`.

### Primary source review

1. **Mumble 1.5.939**, commit `dfbac8d3d9c967003e74b4ffc5610f7d45e8f666`:
   - [`AudioInput.cpp`](https://github.com/mumble-voip/mumble/blob/dfbac8d3d9c967003e74b4ffc5610f7d45e8f666/src/mumble/AudioInput.cpp): microphone and speaker-reference processing are separate; Opus mode selection depends on quality/low-delay settings; echo processing precedes encoding.
   - [`AudioOutputSpeech.cpp`](https://github.com/mumble-voip/mumble/blob/dfbac8d3d9c967003e74b4ffc5610f7d45e8f666/src/mumble/AudioOutputSpeech.cpp): one speech decoder/jitter buffer per speaker, timestamped packet admission, adaptive jitter updates, and Opus null-input PLC when audio is missing.
   - [`Server.cpp`](https://github.com/mumble-voip/mumble/blob/dfbac8d3d9c967003e74b4ffc5610f7d45e8f666/src/murmur/Server.cpp): `processMsg` checks authentication, mute/suppression and sender bandwidth before forwarding to channel listeners. Forwarding is not equivalent to server-side PCM mixing.
   - [User audio settings](https://www.mumble.info/documentation/user/audio-settings/): smaller packets trade overhead for delay; documentation recommends 20 ms for low delay while avoiding some driver problems with 10 ms packets. Echo cancellation is a distinct feature.
2. **Discord official protocol documentation**, commit `f7416a2cf165455f72a31f5ea4e671a4f047f2f7`:
   - [`voice-connections.mdx`](https://github.com/discord/discord-api-docs/blob/f7416a2cf165455f72a31f5ea4e671a4f047f2f7/developers/topics/voice-connections.mdx): Opus 48 kHz stereo over encrypted RTP/UDP; transport encryption remains separate from its DAVE group encryption. Sanser should reuse its own authenticated session/transport, not impersonate Discord or adopt its external service dependency.
3. **WebRTC Audio Processing Module**:
   - [Pinned APM documentation](https://webrtc.googlesource.com/src/+/8c90d57b4151b0a009ce83f85c57dfca8b2e1fe1/modules/audio_processing/g3doc/audio_processing_module.md): acoustic echo cancellation, noise suppression and automatic gain control are processing stages, not codec features. APM can be integrated independently of the complete WebRTC networking stack.
4. **Opus API 1.5**:
   - [Encoder](https://www.opus-codec.org/docs/opus_api-1.5/group__opus__encoder.html): persistent encoder state per stream; microphone speech can use `OPUS_APPLICATION_VOIP`, while mixed/music content has different modes. 48 kHz mono with 20 ms frames means 960 input samples per channel.
   - [Decoder](https://www.opus-codec.org/docs/opus_api-1.5/group__opus__decoder.html): PLC uses null input and an explicit missing duration. FEC is recovered from a subsequent packet; it must fit the playout deadline and must not cause duplicated playback or indefinite waiting.

No third-party source code was copied into Sanser by this investigation.

## Sanser design recommendations

These are engineering choices informed by the sources above, not claims about observed competitor internals.

1. **Keep conversation and system audio separate.** A speaker has its own identity, sequence, capture clock and codec state. Muting shared computer sound must not implicitly mute a conversation; leaving voice must stop microphone capture without ending screen sharing.
2. **Explicit opt-in, muted by default.** Provide Join voice, microphone mute and Leave voice. Permission denial affects voice only. Never automatically open microphones on invitation, reconnect, app launch or screen-share acceptance. Stop capture and queues when sharing ends or the viewer leaves.
3. **Small-room forwarding.** Encode a speaker once and forward its compressed frames to permitted recipients, excluding the origin. Each listener maintains independent decoder/jitter state for every remote speaker. This avoids speaker self-loopback and permits future per-person mute. Do not decode and re-encode every hop unnecessarily.
4. **Treat acoustic echo separately.** Excluding the sender prevents digital self-return, but cannot remove sound from speakers being picked up by the microphone. Full open-microphone use needs a true AEC path with playback reference and device-clock alignment, or an OS voice-processing path. Until that is verified, describe the limitation honestly and recommend headphones/push-to-talk; a simple noise gate is not AEC.
5. **Prevent desktop loopback recursion.** Conversation playback can be captured by the host's system-audio capture and sent to viewers a second time. Native process exclusion, separate endpoint routing or suspending shared system audio while voice is active must address this explicitly; excluding sender voice frames alone does not solve it.
6. **Bound voice delay.** Start with Opus mono/48 kHz/20 ms speech frames and bounded queues; drop expired frames, use PLC for missing deadlines, and prioritize audio above video/repair. FEC is optional and should follow measured loss and its one-frame recovery cost. Video stalls must never stop voice receiving/playout.
7. **Use existing session authorization and authenticated encryption.** A viewer may speak only in its current sharing group, with a verified peer identity. Refuse forged speaker identities, stale generations, replayed frames, oversized payloads and unauthenticated forwarding. Membership removal must cut media forwarding immediately.
8. **Mix only at the output edge.** Sum decoded remote speakers with explicit headroom/limiting into the listener's output; keep own microphone out of local playback. A mixed return path, if implemented instead, must be recipient-specific mix-minus and must not share decoder sequence state among senders.
9. **Measure without recording speech.** Numeric counters are sufficient for mic frames, queue drops, received speech, PLC/FEC, late packets, output underflow and per-speaker levels. Do not write microphone PCM, encoded voice payloads or unrelated app credentials into diagnostics.

## Verification checklist

- Permission denied/unavailable microphone: video and shared sound continue; UI reports voice unavailable and can retry explicitly.
- Host and two or three viewers talk simultaneously: every participant hears every other participant once, and never their own digital return.
- Mute/leave/remove/reconnect: no microphone data after mute/leave; no automatic microphone restart; stale frames from previous membership are rejected.
- Loss/reordering/slow viewer: bounded per-speaker jitter, PLC and queue dropping; no voice backlog, no degradation of healthy participants or video keyframe policy.
- System audio plus conversation: no duplicate voice, feedback loop or recursive loopback; test both directions on physical Windows/macOS devices.
- Video frozen/slow encoder: conversation remains live.
- Headphones versus speakers: separately report acoustic echo behavior; synthetic tests cannot validate AEC or microphone hardware quality.

Research answered the codec/forwarding/permission/queue architecture questions with source and artifact evidence. Actual device AEC, OS loopback exclusion, microphone quality, WAN voice latency and multi-person physical-call behavior require implementation-specific tests; they are not established by this static investigation.
