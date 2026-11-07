# Sanser interface foundation

`tokens.json` defines the shared dark palette and control/card/panel dimensions.
Run `npm run design:generate` after editing it; `npm run design:check` verifies the
checked-in CSS, Dart and C++ outputs. Frontend CI runs the same check.

Consumers:

- Desktop Svelte: `src/styles/tokens.css` and `workspace.css`.
- Mobile Flutter: `design_tokens.dart`, `theme.dart`, device cards and session controls.
- Native floating controls: `native/common/design_tokens.h`, AppKit and Win32.

Blue denotes navigation/actions, green denotes active microphone/connection,
amber denotes pending work and pink denotes stop/leave/error. Status always has a
text label as well as color. Mobile control targets use 48 logical pixels where
space permits; the existing narrow hidden edge handle is retained.

Voice controls preserve listen-only Join and explicit microphone consent. A green
"Microphone on" label uses the reported capture state, not merely the user's
unmute request. Collapsing/dragging does not join voice or change consent. Native
panels remain nonactivating; media capture, transport and decoding stay native.

This first pass uses existing Svelte/Flutter/native components and adds no UI
framework dependency. It establishes the shared visual contract before choosing
larger components from the [GitHub survey](../docs/research/github-ui-platform-survey-2026-11-03.md).
It does not add clipboard, file transfer, full IME, native gamepad or mobile voice.

Local visual exports are in `artifacts/ui-foundation-2026-11-03/`. Mobile exports
use fictitious device/session state and test-loaded fonts; the displayed RTT is
example data. Mac panel exports use a device-free fake voice controller. These
images demonstrate layout, not a real stream or measured performance.

## Session actions (phase 2)

Desktop Active Session groups audio, reconnect, Settings, diagnostics and
Disconnect alongside the selected route and session access mode. Live audio
control is exposed for an SNV2 receiver actually launched with audio enabled;
ordinary sessions now share the same per-viewer mute command as group viewers.
Legacy receivers and receivers started without audio do not expose a misleading
live-unmute action. Changing this viewer's mute does not restart video.

Mobile places sound, keyboard, fit-to-screen and Settings before optional
keyboard/game controls and connection counters. Collapse/hide and Disconnect
stay outside the scrolling list. Very short viewports use the compact control.
View-only sessions expose local pan/zoom without keyboard/game input actions;
input/audio actions are unavailable before the stream is ready. Mobile group
voice remains hidden unless the native engine explicitly advertises support.

A new session gets fresh keyboard ownership. Old audio acknowledgements/errors
cannot update the next session's UI state. No remote monitor selector, OS audio
route selector, clipboard or file-transfer action is advertised by this pass.
These require additional native/protocol capabilities before UI controls.

Validation for phase 2: 273 desktop UI tests, 42 Flutter tests, Svelte/Flutter
analysis and desktop production build. Coverage includes read-only access,
startup capability gates, pending sound commands, stale audio replies and sticky
controls after scrolling. Visual exports with fictitious state are in
`artifacts/session-actions-2026-11-03/`; example RTT is not a measurement.

## Everyday features (phase 3a)

Mobile's text editor now negotiates committed Unicode input with an SNV2 host.
Vietnamese, combining marks and emoji use the existing bounded reliable input
path; old hosts retain the ASCII path, and view-only hosts deny text injection.
The editor prevents duplicate submissions while waiting for the native command.
See [the protocol, platform behavior and remaining phase-3 work](../docs/unicode-input.md).
Phase 3a did not expose clipboard or file transfer.

## Text clipboard (phase 3b)

Desktop adds a capability-aware Text and clipboard panel, including native local
paste/copy and committed Unicode typing. Mobile adds a manual clipboard sheet
under the floating controls. Remote clipboard is opt-in at the host, denied for
view-only sessions and capped at 8 KiB. Received text is reviewed before local
copy; pending operations and previous-session replies cannot update a new editor.
Private payloads are filtered before desktop diagnostics. See [the transport and
validation notes](../docs/text-clipboard.md).

## Consented desktop files (phase 3c)

The desktop controller and host now have a File transfer panel with a native-
capability gate, explicit invitation/accept/decline, progress, cancellation and a
verified save location. An incoming-file banner links to the correct panel from
any workspace page. Received files use Downloads/Sanser with a unique name;
unaccepted invitations create no files. File chunks stay on the bounded low-
priority data lane, independent of input sequences. See [implementation and
validation](../docs/file-transfer.md). This pass is source-only and not released.

## Mobile files (phase 3d)

The floating mobile panel now includes Truyền file with a negotiated capability
gate. Mobile can offer a file selected by the system picker, review and accept
incoming offers, track progress and cancel. Tệp đã nhận in the panel and Settings
preserves access to verified receipts after disconnect, with an explicit native
save/export action. Files use the same Rust worker and protocol as desktop;
video/audio/file bytes never cross Flutter. Full remote IME remains.

## Local IME composition (phase 3e)

Mobile now keeps the IME's draft, selection and candidate changes local until
explicit confirmation and Send. Remote keys cannot interrupt an active candidate;
the complete draft is checked against the native Unicode limit without truncating
emoji or combining marks. Pending input is drained and session-checked before
text is queued; a native rejection retains the draft for review. Desktop text/
clipboard tools now gate operations during browser composition too. See
[behavior, tests and remaining remote IME scope](../docs/unicode-input.md).
Host-side candidate/selection synchronization remains separate and unimplemented.

## Native Mac text mode (phase 3f)

The SNV2 native Mac window adds a capability-gated title-bar Text input action
(Control+Option+I), plus a small local marked-text preview. AppKit owns local IME
candidates; only committed Unicode reaches the host. Physical HID remains the
default and can be restored with the same control. Focus/permission/generation
changes discard candidates; text mode retains balanced shortcut, function-key,
Shift+arrow and Shift+click input. This is not present in the Legacy Mac viewer;
Windows uses the draft composer below. [Behavior and validation](../docs/unicode-input.md).

## Native Windows draft editor (phase 3g)

The SNV2 Windows window now has Text input in its system menu and Ctrl+Alt+I.
A native EDIT keeps IME candidates, selection and corrections local until Send/
Ctrl+Enter. Enter inserts a local newline; candidate input is never an implicit
Send. Escape/Cancel discards the draft, then clicking the stream resumes physical
input. Permission/focus/connection-generation checks and stamped posted messages
keep stale drafts out of a later session. There is no view-only input action.
The UI module/test executable cross-build on Mac, but Windows runtime/MSVC and
real IME/device validation remain before release. No release was made.

## Mobile external keyboard (phase 4a)

The session panel now has an opt-in Bàn phím ngoài action. A focused video/game
surface sends physical keyboard HID events; local fields and sheets keep their
input. Focus/background/readiness changes release held keys, and Ctrl+Alt+Esc
releases all input and disables capture. View-only cannot enable it. Ownership
and bounded repeats reuse MobileInput, with no media work added to Flutter.
See [contract, tests and device checks](../docs/mobile-external-keyboard.md).
This begins phase 4; native controllers and Windows runtime/physical IME checks
are still pending. Network recovery is described below.

## Mobile touch feedback (phase 4b)

Settings and in-session Settings add Rung khi thao tác, off by default and saved
on this phone. Valid click, right-click/drag start and touch game presses request
local feedback; continuous motion, zoom and repeats do not. One in-flight request
and a cooldown bound platform calls. Input never waits for a feedback reply,
and feedback does not assert a host acknowledgement. Read-only/background/
unavailable sessions cannot request remote-touch feedback. See
[the behavior, scheduling and validation](../docs/mobile-touch-feedback.md).

## Adaptive tablet layout (phase 4c)

Wide workspace windows use a navigation rail and readable device-card columns;
narrow split windows retain bottom navigation. Settings is a bounded dialog on
wide windows and a keyboard-aware sheet on smaller ones, with sticky Close.
Floating controls grow modestly on tablet, remain recoverable in short windows
and keep Disconnect outside the scroll area. Text scaling can reduce card
columns; resizing preserves the selected tab and held-key ownership. See
[behavior, preview and validation](../docs/mobile-tablet-layout.md). Real tablet
and native-video placement checks remain before release.

## Network-change recovery (phase 4d)

A native network change on a running mobile session now enters an explicit
recovery state with a visible Cancel action. Accepted-session readiness and
credentials are renewed; Direct candidates are gathered fresh, with existing
Auto/Direct/Relay policy preserved. Attempts are bounded/serialized and stale
media events, queued keys and consent replies are fenced by their generation.
Native video is not considered ready until a new frame. This is a reconnect,
not seamless migration. See [contract and validation](../docs/mobile-network-recovery.md).

## Physical controller keyboard profile (phase 4e1)

An explicit Tay cầm → phím action receives one USB/Bluetooth controller through
the OS and maps sticks/buttons to a fixed keyboard profile. Neutral arming,
per-control ownership, focus/lifecycle and capture-generation guards prevent old
held input from entering another session. Start+Select releases capture. Local
axis motion has a bounded dispatch slot and does not rebuild the session UI.
This is not virtual Xbox/analog/rumble support; the server keeps gamepad: false.
See [scope, mappings and validation](../docs/mobile-controller-keys.md).

## Mobile host shortcuts (phase 4f)

The mobile session adds Phím tắt máy tính: a compact sheet with host-aware
Windows/macOS window and editing actions. Missing host metadata requires an
explicit OS selection. The one-shot chords reuse authenticated HID input with
owned reverse releases, reset epochs, lifecycle/session gates and no automatic
retry. Close remains fixed while action groups scroll. See
[contract, validation and remaining desktop scope](../docs/mobile-host-shortcuts.md).

## Native desktop host shortcuts (phase 4g)

SNV2 Mac and Windows viewer windows now have modeless Host shortcuts controls,
with explicit remote-OS selection and fixed window/editing profiles. The panel
releases physical input on entry and queues whole balanced key groups through
the existing authenticated input lane. Backpressure rejects a whole operation;
no per-action reset may erase an earlier queued shortcut. AppKit UI and UDP
fault tests pass; Windows cross-builds, with runtime testing still pending.
See [scope and validation](../docs/native-session-shortcuts.md).

## Legacy viewer host shortcuts (phase 4h)

The Mac → Windows Legacy viewer now reuses the AppKit panel with only its
Windows-host profile enabled. Signed host capability controls availability;
fixed named actions retain Win-key semantics instead of the physical Mac
Command-to-Ctrl mapping. Bounded reliable operations wait for predecessor ACKs,
and cancellation leaves physical input released until a stream click. See
[compatibility, tests and limits](../docs/legacy-session-shortcuts.md). Both sides
need the new source; no release or installed-app replacement is implied.

## Analog controller core (phase 4i1)

SNV2 now has authenticated single-controller capability and a latest-state
analog path with a separate mouse/gamepad slot, reliable-reset fences and a
neutral/watchdog lifecycle. Windows can use an already installed optional
virtual-controller backend. This increment adds no analog UI or mobile control
mode; the existing keyboard-mapped profile stays available. See
[scope, scheduling, tests and remaining work](../docs/native-gamepad-core.md).

## Mobile analog capture (phase 4i2)

Floating controls add Tay cầm analog, gated by authenticated host support and
exclusive with keyboard modes. Native physical capture sends held state every
40 ms through Swift/C or Kotlin/JNI; Flutter sees status rather than axes.
Neutral arming, capture/attempt generations and focus/lifecycle guards protect
held input. Host capability loss deselects without auto-restart. See
[contract and validation](../docs/mobile-analog-controller.md). Real controller/
Windows driver tests, desktop-native capture and rumble remain.

## Desktop analog capture (phase 4i3)

SNV2 viewers expose Controller in the Mac title bar and Analog controller in
the Windows system menu / Ctrl+Alt+G. Authenticated host capability gates use;
view-only sessions remain excluded. GameController/XInput samples reuse the
mobile native sender. Neutral arming, focus/editor/reset cancellation and a
200 ms stale-poll cutoff prevent indefinite old held states. See
[scope and validation](../docs/native-desktop-controller.md). Real controller/
Windows driver tests, rumble and phase 5 voice/performance validation remain.

## Voice/performance evidence (phase 5a)

Measured native performance now separates audio/voice counts from stage timing.
Recent interval increases identify underflow, concealment, capture drops and
near-full-scale microphone PCM without blaming the network. Baseline/reset/gap
and stale/ended states remain explicit. A guarded offline comparison uses
exported reports and operator-matched conditions; it never merges percentiles
or labels absent evidence healthy. See [measurement procedure and limits](../docs/voice-performance-ab.md).
LiveKit prototyping and physical same-device listening/performance A/B remain.

## Isolated LiveKit voice trial (phase 5b)

The opt-in browser Voice Lab uses the shared design tokens, compact microphone/
deafen/leave controls and desktop/narrow layouts. It requests microphone capture
only after Unmute, clears the entered token, and ends the call on network
reconnection, hidden page or teardown. Numeric WebRTC exports stay separate
from native counters. It does not modify Sanser's native floating bar or voice
transport. See [run instructions, tests and trial constraints](../apps/voice-lab/README.md).
Real endpoint/token/device trials and production integration remain.

Phase 5b1 adds a separate Sanser account login, sharing-role selector and
server-issued trial admission, retaining the manual-token form under a collapsed
advanced section. Join stays listen-only; Leave, late responses and membership
rejection cannot restore microphone consent. Credentials stay in memory. This
off-by-default experiment still needs SFU-enforced revocation, native integration
and actual multi-machine audio validation; it is not a released replacement.

Database admission validation now runs against a disposable PostgreSQL cluster:
all ten API tests pass, including the new voice case. The boolean online-field
decode was corrected. Release remains held pending SFU revocation/media and
real-device voice validation; this does not demonstrate improved voice quality.

Phase 5b2 binds credentials to a persisted room revision and checks admission
after acquiring the group lock. A changed revision leaves the old call with mic
off and asks for rejoin; an unchanged revision keeps the call. The old signed
room grant cannot authorize the new room. Ten PostgreSQL API tests and 35 lab
tests pass, including admission/removal concurrency and stale-response consent
fences. Closing existing SFU media, native integration and acoustic trials remain
unfinished; this is not a complete server revocation or voice-quality fix.

Phase 5b3 adds a durable room-closure outbox and tests it against a real local
LiveKit server with three synthetic Opus peers. Ten seconds of three-way transport
and server-driven disconnection pass, alongside 11 DB API tests and 36 server
unit tests. Cached-token rejoin still succeeds on the self-hosted SFU, so the
strict release gate fails. This is recorded explicitly; neither hardware audio
quality nor production admission is marked complete. See the
[test procedure and remaining gates](../apps/voice-lab/testing/README.md).

Phase 5b4 (2026-11-07) adds the desktop floating WebRTC voice window using the
Lab's shared consent/session adapter. The host has a combined Stop sharing/voice
bar; viewers have voice controls only. A native acknowledgement stops the old
mic/player and pauses shared system sound before the window opens. Join is
listen-only; closing restores shared sound, never mic consent. A bounded startup
watchdog also restores sound if the voice webview fails to connect. Enhanced
voice currently supports one active desktop session at a time.

The private-SFU admission gateway now rejects both cached and actual
SFU-refreshed tokens after revocation; the real local three-peer gate passes.
This supersedes the prior 5b3 replay failure. Desktop UI, Rust, server/DB and
synthetic native transport checks pass. Physical WebView/mic/speaker/focus,
Windows runtime and WAN tests remain; no audible-quality claim or release is
made. See [deployment, behavior and pending acceptance](../docs/livekit-voice-gateway.md).

Phase 5b5 bounds microphone permission/publication waits to 15 seconds and
disposes late results. Retired mic events cannot turn off a replacement; output
device-change recovery never acquires a microphone. Closed-round retention is
bounded and preserves pending/live/leased work. A real local three-peer Opus
sample ran for 120 seconds with heartbeats and authorization active, without a
three-second track stall. Desktop UI and local DB/SFU checks pass. Linux deployment
templates are prepared; public media/TURN and physical speech/jack/WebView tests
are still unverified, so release remains held at 2.3.3.
