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
or reconnection. Version 2.2.19 combines the host's screen-sharing
indicator and voice controls into one compact dark floating bar. The host has a
screen icon, a microphone shortcut, an expand arrow and a red Stop button; viewers
only have a **Group voice** bar, without screen-sharing or Stop controls. Click the
arrow or title to expand the icon buttons for Join/Mute/Deafen/Leave without returning
to the main application. Icons have tooltips and accessible action names. The bar
can be dragged and does not take keyboard focus. Joining is still listen-only;
the microphone shortcut becomes available after joining. Stop sharing also leaves
voice. The sharing indicator and Stop remain available when voice is unsupported.
Switching pages or collapsing the bar does not leave
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
mute and deafen, not per-person mute controls, a member/speaking list or push-to-talk.
Version 2.2.19 adds SpeexDSP acoustic echo cancellation (AEC), using actual
rendered system sound plus conversation as a reference before microphone Opus
encoding. This is distinct from PLC, noise gating and automatic gain control.
Physical speaker/microphone routes still need verification on both machines.

## Group voice playback and panel update after 2.2.19

The next source update replaces the expanded icon grid with a single Join action
before joining, then labeled Unmute/Mute mic and Mute/Hear voice controls after
joining. Host and viewer bars retain separate roles. The shorter collapsed bar
includes a labeled Join/Muted/Mic on shortcut; actual capture confirmation controls
the live indicator. Leaving restores Join. No participant count or speaking status
is invented from packet reception.

All remote speakers now feed one 5 ms output sample clock. Independent decoder
and jitter state remains per source. Adding two or three speakers with staggered
arrival times must not add output PCM blocks to the same playback interval.
See [reproduction and before/after measurements](investigations/group-voice-mix-clock.md).

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

Digital exclusion does not itself cancel acoustic echo. The AEC reference combines
native rendered system/voice PCM and the original local host system capture.
Full Windows endpoint loopback takes precedence over app playback, so a fallback
cannot count the same voice twice. Reference PCM is bounded to one second in RAM;
no PCM is written to logs/disk. Missing/stale or silent reference uses dry speech
with the same one-block (5ms) delay as Speex preprocessing. Brief reference holes
do not reset the overlap/add history or insert a fresh silent block; a longer
reference gap resets acoustic adaptation separately. Wet/dry changes crossfade
on the aligned timeline. Mute/Leave destroys adaptation and delayed microphone
PCM. Capture blocks carry their
first-sample device timestamp; render references use actual queue start time on
Mac and WASAPI padding on Windows. The causal 200ms filter learns acoustic delay;
no fixed 40ms reference shift prevents cancellation of short paths.
Playback by another independent process, Bluetooth latency/device drift and
nonlinear speaker clipping still need physical tests.

## Validation

`sanser-group-voice-test` uses synthetic PCM and three encrypted UDP loopback
viewers. It covers strict consent parsing, Opus duration/packet bounds, sequence
overflow, duplicate/old epoch rejection, loss/FEC attempts, bounded stale dropping,
per-source mixing/mute, priority under video/audio load, default microphone-off,
three-viewer forwarding without own-voice return, deafen, a host not participating,
and a nonvoice viewer. A deterministic 7ms worker cadence also verifies 5ms audio
blocks catch up instead of reducing the PCM output rate. `sanser-voice-echo-test`
uses synthetic mixed system/voice output, delayed multipath echo at 0/10/20/30/60/100ms,
independent near-end speech, batched render callbacks and stale-reference safety.
It also covers silent/intermittent references and the fixed processing timeline.
Single-speaker in-range PCM is checked against an independent Opus decoder;
overloaded sums use stateful Opus soft clipping instead of applying tanh to all
speech. `SNVOICE_HEALTH` reports bounded numeric clipping, queue, PLC, AEC and
playback counters without storing speech or device/speaker identity.
It opens no microphones/speakers. Existing audio/video transport tests remain applicable.
Windows also tests process-exclusion activation/cancellation ownership without
opening an endpoint. UI/Rust tests exercise scoped commands, capability fallback,
permission/device statuses and leave racing an in-flight unmute.

No development test opens a real microphone, records speech or plays hardware
sound. Physical Windows/macOS calls still need verification: permission denial,
headsets/Bluetooth changes, voice plus shared music, simultaneous speakers,
loss/WAN/relay, device sleep, page changes and no microphone after Leave. Synthetic
tests and static REA research cannot establish hardware sound quality or AEC.

`sanser-voice-panel-test` uses a fake voice controller to check host/viewer controls,
listen-only Join, explicit Unmute, the compact Mute shortcut, collapse without
leaving, Deafen, Leave, Stop sharing and the sharing-only fallback. It opens no
audio devices. On macOS, setting `SANSER_PANEL_PREVIEW_DIR` to an existing directory
exports images of the test panel itself for visual inspection; it does not capture
the screen or other applications.

Research and primary sources: [group voice investigation](investigations/group-voice-research.md).

## 2.2.20: multiple talkers and compact controls

The mixer emits one shared playback tick for staggered remote talkers, rather
than emitting extra partial mixes when their packet deadlines differ. Separate
speaker decoders, jitter reserves and PLC remain. This fixes a reproduced
scheduling defect; real speaker/microphone quality still requires listening tests.

The compact floating bar exposes Join before joining and an explicit mic state
and toggle afterward. Expanded controls distinguish Unmute/Mute mic, Hear/Mute
voice and Leave. Hosts retain the combined sharing bar; viewers retain voice-only
controls. Join still listens only and Unmute explicitly opens capture.
