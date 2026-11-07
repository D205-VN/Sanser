# Native desktop shortcut panel — phase 4g

Source increment, 2026-11-05, Sanser 2.3.3. No release, install or device benchmark.

## Use and supported engines

The **SNV2 native viewers** have a local **Host shortcuts** panel:

- On Mac, click **Shortcuts** in the viewer's title bar.
- On Windows, use the viewer's system menu → **Host shortcuts**, or
  **Ctrl+Alt+K**. AltGr+K is not treated as this local shortcut.

Choose **Windows host** or **Mac host**, select a listed action, then explicitly
press **Send shortcut**. No remote OS is assumed from the controller's OS.
The profile matches the mobile [Windows/Mac shortcut sheet](mobile-host-shortcuts.md):
switch app, minimize/close, host-specific window actions and common editing.
Copy/Paste uses the host's clipboard; the panel never reads the local clipboard.
The status says queued, not that the target application accepted the request.

The shared SNV2 host must have authenticated and announced its existing control/
Unicode capability. Input must be enabled and unpaused. View-only/group viewers
have no action panel. Phase 4h separately adds the panel to the
[Legacy Mac → Windows viewer](legacy-session-shortcuts.md), using a capability-
gated fixed operation rather than SNV2 HID groups. No engine selection is changed.

Opening releases physical input and discards active local text composition.
Close the panel, then click the stream to resume physical input. Host choice is
cleared on each open. Inactive focus, permission loss, capture/input pause,
generation change and viewer teardown invalidate the panel. Native controls from
a closed Mac panel cannot send or cancel actions in its replacement.

## Scheduling and input safety

The Mac panel uses a modeless NSPanel and a scrolling native table; Windows uses
a modeless child panel with a native list. Neither adds a modal dialog/menu loop
or waits for a network ACK. Video/audio continue using the existing native
workers. The main loop still consumes frames and refreshes panel availability.
Windows keyboard navigation joins the existing bounded window-message pump;
the list scrolls and Close/Send stay separate. A very small Windows viewer must
be enlarged before opening; resizing below 320×240 client pixels closes the panel.

`ShortcutSession` is a UI-thread model with a fixed action allowlist and explicit
host selection. `Peer::shortcut` accepts at most four unique supported keyboard
usages, rechecks negotiated permission/readiness and the expected remote epoch,
then queues the whole down/reverse-up group under the same sender lock as normal
input. Encryption, reliable sequence reservation, pending-ACK state and scheduler
publication happen as one bounded transaction. Insufficient space refuses the
whole group **without consuming sequences or terminating the media session**.
Control/audio/video retain their existing scheduler lanes. There is no retry of
a rejected *operation*; accepted reliable packets keep normal loss/ACK recovery.

The reset is sent **once on panel entry**, using the same bounded preflight.
There is no reset between or after actions: host InputWorker treats reset as a
queue fence and discards older pending work. A reset at the end of a chord, or
before every rapid action, could erase a valid Copy/Paste sequence on a slow
injector. Balanced releases retain the actions in order. Existing input lease,
disconnect and timeout recovery continue to release state on failure.

No new wire type, arbitrary command execution, remote window enumeration,
UAC bypass or virtual-controller backend is introduced. OS/app shortcut overrides,
keyboard layouts and elevated-app/Secure Desktop permissions still apply.

## Verification

- Seven targeted native suites pass: shortcut model/scheduler, authenticated UDP
  text/shortcuts, transport, input engine, user messages, AppKit shortcut UI and
  AppKit IME regression.
- Real loopback peers exercise lost/duplicated panel-entry reset, two consecutive
  chords, balanced order/no duplicate edges, stale epochs, capture pause, older/
  denied/sharing hosts and a full 64-packet reliable window. A declined action or
  panel reset leaves media alive and later input still reaches the host.
- The native Mac client builds and links with both the legacy and SNV2 view code.
  AppKit panel tests use fake callbacks, not OS input injection or screen capture.
- Portable model/scheduler tests also pass ASan+UBSan.
- Windows panel/test executables cross-compile with the existing LLVM-MinGW
  toolchain, including `-Wall -Wextra -Werror`; integrated backend syntax-checks.
  **Windows executables have not run on this Mac.** CMake registers tests in both
  the standalone Windows UI job and the full desktop test build.

Actual Windows/MSVC/runtime UI, high-DPI/multiple-monitor behavior, target-app
shortcuts, elevated apps and real two-computer latency remain to validate. This
does not complete virtual Xbox/analog/rumble or phase 5
voice/performance A/B work. No measured latency reduction is claimed.

Primary API references: [Apple NSPanel](https://developer.apple.com/documentation/appkit/nspanel),
[Microsoft modeless dialog/message processing](https://learn.microsoft.com/en-us/windows/win32/dlgbox/using-dialog-boxes).
