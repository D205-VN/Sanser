# Mobile tablet layout — phase 4c

Source increment, 2026-11-05, Sanser 2.3.3. No release or device installation.

## Adaptive workspace

The signed-in workspace uses a left navigation rail when the available window
is at least 840 × 600 logical pixels. Smaller windows retain bottom navigation;
rotating a phone into a short landscape window does not force a tablet rail.
Selection survives resizing and split-screen transitions. The rail can scroll
when a keyboard or large text leaves little vertical room.

Computer cards use two natural-height columns when the content width can fit
two readable cards. Large text increases the minimum card width and can restore
one column. No fixed aspect ratio clips a long device name. Computer content is
capped at 1,080 logical pixels; other workspace pages at 720.

An active remote session retains its existing full-screen video/input surface.
It does not move into the workspace rail or restart the native engine when the
window resizes. Text-input ownership and read-only capability gates stay intact.

## Session controls and settings

Expanded floating controls can grow from 320 pixels on a phone to 400 on a
large window, with a height capped at 600. The panel stays draggable, collapsible
and hideable. Disconnect stays outside its scrolling content. Normalized anchor
coordinates are clamped against the new available bounds after resize/rotation
or Scaffold keyboard avoidance. Compact height respects text scaling; very
short windows retain a recoverable compact control rather than an overflowing
expanded panel.

In-session Settings opens as a centered dialog on a wide window (maximum width
680, height 760), and a bottom sheet on smaller windows. Both keep a fixed Close
action above scrolling settings. The sheet avoids keyboard insets and system
safe areas. An already-open route adapts within its existing dialog/sheet when
resized; it is not replaced and does not issue session commands.

Settings dropdowns now expand within the available width and ellipsize long
option labels instead of overflowing. Their complete text remains in the widget
semantics; this does not change the selected transport or saved values.

## Validation and limits

Seven new widget tests cover rail/tab preservation, card columns and large
text, dialog/sheet keyboard avoidance, sticky Close/Disconnect, hidden controls,
very short windows, and a held external Shift across a live-session resize.
The same test verifies view-only input actions stay absent after layout changes.

All 90 Flutter tests pass, `flutter analyze` reports no issues and an isolated
unsigned iOS Release builds (23.6 MB Runner.app). Preview PNGs in
`artifacts/mobile-tablet-2026-11-05/` render production widgets with fictitious
account/devices and blank video; example RTT/FPS values are not measurements.

Actual iPad/Android tablet split-screen, floating keyboard, hardware keyboard,
accessibility text settings and native-video placement still need device tests.
This pass does not add controller support, alter iOS
orientation policy or establish a streaming-performance improvement.
[Network-change recovery](mobile-network-recovery.md) is a separate increment.
[Physical controller → keyboard](mobile-controller-keys.md) is also separate;
it does not implement virtual Xbox/analog injection at the host.
