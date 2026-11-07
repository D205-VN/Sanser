# Mobile analog controller — phase 4i2

Source increment, 2026-11-05, Sanser 2.3.3. No release or phone installation.

## Use

In a streaming control session, open floating controls and choose **Tay cầm
analog**. Connect one USB/Bluetooth controller through the phone OS. Release
buttons/triggers and center both sticks to arm. Android attachment alone is not
a hardware sample: move/release a control if still waiting. **Start+Select**
releases capture locally without forwarding the chord.

The action requires the new mobile implementation and authenticated per-session
host capability. Currently its production provider is the optional Windows
backend in [the native core contract](native-gamepad-core.md). Older/unsupported
hosts keep it disabled and show the keyboard alternative. View-only/group
viewers cannot enable it. No driver is installed here; macOS hosts do not
advertise analog. Generic server registration remains `gamepad: false`; live
SNV2 negotiation gates this action instead of an unverified registration flag.

Analog preserves stick/trigger magnitude without emitting WASD/arrows. It is
exclusive with **Tay cầm → phím** and external keyboard capture. The fixed Xbox
layout includes A/B/X/Y, shoulders, Start/Select, thumbstick presses and D-pad;
Guide stays local. Rumble, multiple players, gyro and custom profiles remain.

## Native lifetime and scheduling

iOS GameController/Android input snapshots go directly through Swift/C or
Kotlin/JNI to C++. Neither axes nor heartbeat packets travel through Dart.
Flutter receives connected/locally-armed/paused transitions and periodic host
availability, rather than every analog sample.

C++ validates finite ranges, maps 14 buttons and scales signed 16-bit sticks/
8-bit triggers. One latest state sends every 40 ms while held, with an 8 ms
minimum interval for new state. Neutral arming matches host thresholds and
starts from canonical zero. Status means locally armed/queued, not game/driver
acceptance. There is no history queue or Flutter heartbeat timer.

Session/media-attempt guards and monotonic capture IDs reject old enables,
stops and samples. Native capture checks remote generation/capability on every
pulse. Start first drains/resets old Flutter input. Editors/settings/files,
pointer/game mode changes, focus/background loss, Reset, teardown and recovery
stop capture. Capability loss deselects analog; restoration cannot auto-enable it.
Device replacement must arm again. Invalid state, Start+Select or rejected send
stops the heartbeat. Best-effort disconnect plus the host's 250 ms lease handles
a lost final release. Already transmitted packets cannot be recalled; a blocked
OS injection cannot be preempted. A long network stall can require neutral again.

## Verification and limits

- Full Flutter suite: **133 passed**, analyzer clean. Six new cases cover bridge
  mode, status-only traffic, delayed Reset/enable cancellation, stale replies,
  paused/closed streams, capability loss without restart, exclusive UI, editor
  suspension and view-only refusal.
- Portable native tests cover all button mappings, axis endpoints/triggers,
  malformed samples, neutral/attach, held heartbeat, capture/epoch fences,
  escape and send rejection; ASan+UBSan pass.
- Mobile C ABI authenticated loopback verifies heartbeat beyond the host lease,
  Reset/stale-stop isolation, capability withdrawal and view-only refusal,
  alongside existing video/input/text/clipboard/file tests.
- Isolated unsigned iOS Release builds with updated Swift/C header/static
  library. Kotlin compiles against Android 36/Flutter/JVM 17. JNI receives a
  host syntax check with a JDK AttachCurrentThread signature adaptation;
  **this is not an Android NDK/APK build**.

Physical iPhone/Android/vendor controllers, OS-reserved controls, Windows driver/
game behavior, school Wi-Fi and actual latency remain untested here. No driver
or physical device was exercised. Native desktop analog capture, rumble, macOS
virtual-controller hosting and phase 5 voice/performance A/B remain separate.

## Shared desktop follow-up

Phase 4i3 extracts the mapper/sender to `native/desktop/controller_capture.h`;
`native/mobile/gamepad_capture.h` preserves the mobile API through an alias.
Mobile does not acquire the desktop UI-poll freshness deadline: its OS adapters
retain their own capture/lifecycle rules. Both native mobile suites pass after
extraction. See [native desktop capture](native-desktop-controller.md) for the
new SNV2 desktop controls and remaining device validation.
