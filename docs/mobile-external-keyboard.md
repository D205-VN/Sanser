# Mobile external keyboard — phase 4a

Source increment, 2026-11-05, Sanser 2.3.3. Not a release or a physical-device
validation result.

## Use

Connect a USB/Bluetooth keyboard to the phone/tablet, start a control session,
open the floating controls and enable **Bàn phím ngoài**. Tap the video to give
it keyboard focus. **Ctrl+Alt+Esc** releases held input and switches this mode
off. The existing touch and keyboard-mapped game controls remain available.
Each new session starts with external capture off. View-only sessions have no
external-keyboard action; startup/unavailable sessions cannot enable it.

Only a focused video/game surface forwards keys. Settings, clipboard, file
sheets and the local Unicode composer are outside its focus subtree. Opening
a sheet, losing focus, losing input readiness, switching sessions or leaving
the app releases this source's held keys. After returning from the background,
tap the video to focus it again. There is no global keyboard hook.

## Input contract

Flutter physical USB HID keyboard-page usages use the existing reliable native
input path: letters, numbers, supported punctuation, navigation/function/keypad
keys and left/right Shift/Ctrl/Alt/Meta. The allowlist matches
`native/desktop/keyboard.h`. Consumer/media/system usages and gamepad/D-pad
device events stay out of this path. OS-reserved shortcuts may never reach the
app; actual iOS/Android delivery needs device testing.

Physical presses use the host's keyboard layout. The handler does not also send
`KeyEvent.character` as text. For local Telex/Pinyin/Japanese candidates and
Vietnamese/emoji commits use **Bàn phím** and its explicit Send workflow. This
feature does not synchronize the host's IME selection or candidate window.

Each press has a separate ownership token. Focus loss cannot release a Shift
still owned by a touch/game control. Synthesized downs and repeats without an
owned down are ignored; a synthesized up can release an owned key. Only one
repeat per press can wait on the native bridge at a time. Modifier/lock keys
are not repeated. A global reset revokes ownership, so later repeat events
cannot resurrect the old press. The emergency shortcut intentionally resets
all input owners and clears the toolbar modifier indicators.

No video/audio data crosses Flutter and no protocol, native dependency or
platform signing setting changes are required by this increment.

## Verification and remaining work

`apps/mobile/test/external_keyboard_test.dart` covers HID filtering, ordered
down/repeat/up, modifier ownership, repeat backpressure/reset, emergency release,
read-only/readiness gates, local sheet isolation, background and session swaps.
`session_actions_test.dart` exercises the actual floating opt-in action and
shortcut. These use simulated Flutter events and fake engine calls.

Local checks on 2026-11-05: all 75 Flutter tests pass, `flutter analyze` reports
no issues, and an unsigned iOS Release builds in an isolated project copy
(23.5 MB Runner.app). No phone installation, Windows runtime result, measured
latency improvement or Android APK build is claimed by these checks.

Before release, test USB and Bluetooth on Android/iPhone/iPad against Windows
and Mac hosts: Shift+A, Shift+click, Ctrl/Command shortcuts, AltGr, arrows/F-keys,
disconnect while held, app switching, local IME, rotation and session retry.
Bluetooth disconnection handling also depends on whether the OS emits key-up
or focus/lifecycle events; the emergency shortcut and existing release action
remain available. [Optional touch haptics](mobile-touch-feedback.md) is now a
separate source increment, as is [adaptive tablet layout](mobile-tablet-layout.md).
[Physical controller → keyboard](mobile-controller-keys.md) now has its own
opt-in profile. Virtual-controller support (`gamepad: false`) remains separate.
[Network-change recovery](mobile-network-recovery.md) now invalidates queued
commands from the old media revision before opening another engine.
