# Mobile → desktop shortcuts — phase 4f

Source increment, 2026-11-05, Sanser 2.3.3. No release or hardware validation.

## Use

In a streaming control session, open floating controls → **Phím tắt máy tính**.
The sheet matches `active.hostDeviceId` to the device list and reads that host's
declared platform. It never guesses from the phone OS, device name or another
online device. Missing/unknown platforms require an explicit Windows/macOS
selection before actions appear. The selection is local to this sheet; it does
not modify device registration or change the host's OS.

Buttons have an icon, action name and separate shortcut caption. The header's
Close remains outside the scrolling content. Read-only sessions have no action;
connecting/reconnecting/unavailable sessions cannot open it. A loss of readiness
while the sheet is open disables its controls. No key is sent merely by opening
the sheet or selecting a platform. Opening/closing intentionally resets held
input; tap the video again to resume hardware keyboard/controller capture.

| Action | Mac host | Windows host |
| --- | --- | --- |
| Switch app / previous app | Command+Tab / Command+Shift+Tab | Alt+Tab / Alt+Shift+Tab |
| Minimize | Command+M (current window) | Win+M (all windows) |
| Minimize this app's windows | Option+Command+M | — |
| Fullscreen / hide app / search | Control+Command+F / Command+H / Command+Space | — |
| Desktop / task overview | — | Win+D / Win+Tab |
| Snap left / right | — | Win+Left / Win+Right |
| File Explorer | — | Win+E |
| Close active window | Command+W | Alt+F4 |
| Copy / paste / cut | Command+C / V / X | Control+C / V / X |
| Undo / redo | Command+Z / Command+Shift+Z | Control+Z / Control+Y |
| Select all / find / save | Command+A / F / S | Control+A / F / S |

Copy/Paste here uses the **host clipboard**. It does not automatically read or
write the phone clipboard; use the existing consented Clipboard sheet for that.
These are ordinary physical keyboard requests to the foreground host app. OS
settings, keyboard layout, application shortcut overrides, input permissions and
secure desktops still apply. A queued result does not prove that the target app
accepted the action. There is no force-quit, Secure Attention Sequence, UAC bypass,
window enumeration or arbitrary command execution.

## Input contract

`MobileInput.chord` validates and snapshots at most four unique HID keys using
the native keyboard allowlist. It refuses overlapping shortcut requests or keys
already held by another source. A new per-operation owner presses keys in order
and releases them in reverse order, preserving another owner's key if acquired
concurrently. This reuses the existing authenticated reliable input lane and
host-side control permissions, including legacy-compatible HID behavior; no new
wire protocol, native permission or media pipeline is introduced.

The sheet suspends hardware-controller capture and unfocuses local input before
opening. While awaiting native replies, actions and platform selection stay
disabled. Close/background/reset/session change cancels subsequent presses.
Every global reset and bridge failure advances an input epoch, so returning to
ready cannot resume an old half-sent chord. Native errors reset input; there is
no automatic retry after an ambiguous result. Releases remain subject to the
existing session fence; recovery/disconnect supplies its own reset.

## Verification and remaining scope

Twelve new tests exercise every profile's exact HID press/reverse-release order,
strict platform lookup, invalid/read-only/held-key rejection, concurrent taps,
reset/failure/key-up errors, ownership, immutable snapshots, lifecycle resume,
close-during-send, unknown/matched-host UI, pending/disconnected sessions and
short landscape with large text. The full Flutter suite passes **127 tests**;
analyzer reports no issues. The isolated unsigned iOS Release builds (23.7 MB).
No personal Xcode signing configuration or packaged desktop binary changed.

Real iPhone/Android → Windows/Mac tests remain: normal/elevated apps, fullscreen,
non-US keyboard layouts, host-customized shortcuts, closing a window with an
unsaved document, keyboard/controller capture resumption and network failure
while a modifier is down. This is **mobile-to-host shortcut UI**, not a new action
panel in the native desktop viewers. The separate
[SNV2 native desktop panel](native-session-shortcuts.md) now implements the fixed
profiles there. [Legacy viewer actions](legacy-session-shortcuts.md) now have a
separate capability-gated Windows implementation. Virtual gamepad/analog/rumble backends and
roadmap phase 5 voice/performance A/B remain.

Mappings reference primary OS documentation:
[Apple Mac shortcuts](https://support.apple.com/en-gb/102650),
[Microsoft Windows shortcuts](https://support.microsoft.com/en-au/accessibility/windows/keyboard-shortcuts-in-windows).
