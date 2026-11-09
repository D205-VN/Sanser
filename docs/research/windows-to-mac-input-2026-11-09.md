# Windows → Mac input review, 2026-11-09

## Confirmed defects and changes

- Windows forwarded physical Control+C/V; macOS applications normally require
  Command+C/V. A Windows-only mapper now temporarily releases physical Control,
  sends the native Command chord, and restores only still-held Control sides.
  It activates only after an authenticated Mac host capability is received.
  Windows/unknown hosts, Shift/Alt/Command combinations, games' other Control
  keys and Ctrl+click retain their native meaning. Focus/reset, repeats, both
  Control sides, early Control release and host-generation changes are fenced.
- A checked **Mac Copy/Paste** item in the Windows viewer's window system menu
  disables this convenience mapping when literal Control+C is needed, such as
  interrupting a process in Terminal. The default enables Mac copy/paste.
- Windows sub-notch wheel messages were truncated to zero. Independent signed
  axis remainders now accumulate them. Horizontal wheel messages now use scroll
  axis 1; Mac injection converts Windows right-positive direction correctly.
  Unsupported axes are rejected; focus loss clears fractional remainders.
- Releasing one mouse button previously released Windows capture even while
  another remained held. Capture now stays until the last button is released;
  stolen capture releases remote input rather than leaving a drag stuck.
- Mac keyboard events now mark held-key autorepeat. Caps Lock uses native
  flags-changed events and toggle state, without toggling again on repeats.
  Committed Unicode text remains literal even when Caps Lock is on.
- Opening the shortcut/gamepad controls clears the Windows mapping state after
  the transport's authenticated input reset, avoiding stale local modifiers.

## Review and regression coverage

| Operation | Coverage |
| --- | --- |
| Ctrl+C/V, left/right/both Ctrl, repeat, early release | Portable event trace plus actual CGEvent construction/injection callback |
| Raw Control, Shift/Alt/Command chords, Ctrl+click | Portable regression and Windows window-message fixture |
| Letters, digits, punctuation, navigation, F1–F12, keypad, modifier sides | All entries in the existing HID-to-Mac key table; key-down/up mapping |
| Caps Lock and held typing | Native Mac flags and autorepeat assertions |
| Unicode accents/emoji/newline/tab | Existing text, scalar validation and local IME regressions |
| Left/right/middle click, double-click, drag, display edges | Native Mac event assertions; Windows multi-button/capture fixture |
| Vertical/horizontal/high-resolution wheel | Accumulator, packet validation, Windows messages, native Mac axis assertions |
| Focus loss, released input, read-only, disconnect | Existing native input/permission/recovery tests plus capture and mapper reset cases |

These tests capture generated OS events rather than typing into the user's
applications. Physical Windows → Mac tests in TextEdit, browser text fields,
Finder and the reported game still need a real two-machine session. Constructed
events do not prove acceptance by every application or protected game.

## Keyboard and clipboard semantics

Ctrl+C/V uses the **remote Mac clipboard**. It does not silently read or replace
the Windows clipboard; cross-device text transfer remains an explicit existing
clipboard feature with host permission.

Windows Alt+F4/Alt+Tab and Mac Command+W/Command+Tab are different OS shortcuts;
local Windows can also reserve system combinations. Use the viewer's **Host
shortcuts** control (Ctrl+Alt+K, select **Mac host**) for close/minimize/switch,
select-all/cut/undo/redo/find/save. Ctrl+Alt+I opens local Unicode composition;
Ctrl+Alt+Escape releases remote input. Physical keys otherwise follow the Mac
keyboard layout. This change does not implement a keyboard driver or bypass
macOS Accessibility permission or game protections.
