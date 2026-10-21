# Group voice in multi-computer sharing

Group voice was introduced in **2.2.17**. The floating controls described below
are included in **2.2.18**, with engines advertising `groupVoiceFloatingControls: true`.
Both host and participants need engines advertising
`groupVoiceImplementation: true`; old engines still share video/system sound but
voice controls explain that an update is needed.

## Using it

1. Start **Share with multiple computers**, invite viewers and let them join.
2. On the host's sharing panel or a viewer's Active session, choose **Join voice**.
   This opens listening only. It does not open a microphone or request permission.
3. Choose **Unmute mic** to allow microphone access and talk. Use headphones.
4. **Mute mic** closes capture; **Deafen voice** stops listening and mutes the mic;
   **Leave voice** closes both. These controls do not stop sharing video.

The room supports the host and up to three viewers. Consent is scoped to the
current native group/session, is not saved, and is not restored on a new engine
or reconnection. Updated engines show a floating **Sanser · Voice** button on
both host and viewer. Click it to expand Join/Mute/Deafen/Leave without returning
to the main application. Switching pages or collapsing the button does not leave
the call; **Leave voice**, disconnect or ending sharing stops participation. Old
engines without floating controls retain leave-on-navigation for safety.
The UI waits for native state confirmation after Join/Mute/Leave, rather than
treating a successful command write as acknowledgement. An older idle status
must not undo a Join that is still being processed, and Mute/Leave must not label
the microphone off before native confirmation.
A native engine exit closes
capture; a failed/denied microphone does not terminate screen sharing. Default
microphone changes reconnect after prior explicit consent; Mute/Leave cancels it.

Shared system sound and conversation have separate controls. Muting the shared
computer's sound does not mute voice. The first version has local microphone
mute and deafen, not per-person mute controls, a member/speaking list, push-to-talk,
noise suppression, automatic gain control, or verified acoustic echo cancellation.
Headphones reduce speaker-to-microphone feedback; Opus PLC is not echo cancellation.

## Transport and audio

Microphone capture uses AVFoundation permission + Core Audio input on macOS and
WASAPI eCapture/eCommunications on Windows. Neither path uses system loopback as a
microphone. Capture converts/resamples to 48 kHz and hands bounded 5 ms PCM blocks
to a voice worker; real-time device callbacks do not encode or send network data.

Each talker owns one mono Opus speech encoder (32 kbps, 20 ms / 960 samples,
constrained VBR, in-band FEC requested, DTX). There is no third-party voice service,
new listening port, cloud recording, or extra codec installation. No microphone
PCM or encoded speech is written to diagnostics or disk by this feature.

The sharing host forwards compressed voice between permitted viewers, excluding
the origin, and sends its own microphone as speaker 0. Viewer slots 1–3 are
assigned by the authenticated host. A viewer cannot claim another slot. Host
participation is optional: it can forward viewer conversation without listening
or opening its own microphone. Removal ends that membership's forwarding.

`SVO1` voice records travel inside the existing authenticated/encrypted SNV2 Audio
packet, with a voice epoch, source slot and sequence separate from screen sound.
Payloads are bounded to 400 Opus bytes; receivers validate 20 ms duration, speaker
identity, generation/replay protection and rate limits. Authenticated `SVC1`
keepalives announce join/mic/deafen; a two-second lease and repeated preferences
prevent an indefinitely stale permission after a lost toggle. `SVI1` assigns the
viewer slot. Local mute gates capture/queues without waiting for remote ACKs.

Every listener has one Opus decoder and bounded jitter queue per remote source.
A 20 ms initial reserve permits reorder/FEC attempts, missing deadlines use PLC
for at most three packets, and old speech is discarded. The output mixes decoded
speakers with a smooth limiter. The independent bounded voice TX lane follows
control/input and precedes screen audio/video/repair, with a 40 ms send freshness
limit. Video pause/repair does not gate voice. These budgets are software policy,
not measured end-to-end latency guarantees.

## Preventing digital loopback

ScreenCaptureKit excludes the current host process's playback from system sound.
On compatible Windows builds the host uses process-loopback exclusion for its
own process tree. This is actual `ActivateAudioInterfaceAsync` process exclusion,
not a promise based on the OS version. If unavailable or not successfully started,
shared system sound is suppressed while the host listens to voice, then resumes
after Leave/Deafen. This avoids recapturing and retransmitting conversation.

This does not cancel acoustic echo, nor promise to exclude playback by another
independent Sanser process (for example simultaneously viewing another host).
Test simultaneous sharing/viewing and Bluetooth routes separately.

## Validation

`sanser-group-voice-test` uses synthetic PCM and three encrypted UDP loopback
viewers. It covers strict consent parsing, Opus duration/packet bounds, sequence
overflow, duplicate/old epoch rejection, loss/FEC attempts, bounded stale dropping,
per-source mixing/mute, priority under video/audio load, default microphone-off,
three-viewer forwarding without own-voice return, deafen, a host not participating,
and a nonvoice viewer. Existing audio/video transport tests remain applicable.
Windows also tests process-exclusion activation/cancellation ownership without
opening an endpoint. UI/Rust tests exercise scoped commands, capability fallback,
permission/device statuses and leave racing an in-flight unmute.

No development test opens a real microphone, records speech or plays hardware
sound. Physical Windows/macOS calls still need verification: permission denial,
headsets/Bluetooth changes, voice plus shared music, simultaneous speakers,
loss/WAN/relay, device sleep, page changes and no microphone after Leave. Synthetic
tests and static REA research cannot establish hardware sound quality or AEC.

Research and primary sources: [group voice investigation](investigations/group-voice-research.md).
