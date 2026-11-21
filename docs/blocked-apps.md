# Blocked apps (host)

Since 2.3.19 a host can list applications that the person connected to it must
not see or use. Websites are not covered yet; block the whole browser instead.

## Settings

Host page and Settings → Host → **Blocked apps**. The list is stored in this
computer's preferences (`preferences.host.blockedApps`, at most 64 entries) and
is passed to the host engine at launch as repeated `--blocked-app <id>`
arguments, for one-to-one sessions and multi-viewer shares. No viewer message
can change it. Changes apply from the next connection.

- **Choose from running apps** lists regular apps on this computer: macOS
  through NSWorkspace (JavaScript for Automation, no Automation permission),
  Windows through `tasklist`. Nothing is started or stopped.
- An app can also be typed: a macOS bundle identifier (`com.google.Chrome`) or
  a Windows program file name (`chrome.exe`). Matching ignores case.

## What happens during a session

| | macOS host | Windows host (SNV2 and Mac-viewer legacy) |
| --- | --- | --- |
| Shared screen | The app's windows are excluded from ScreenCaptureKit by window ID; the viewer sees what is behind them. A CoreGraphics window list is polled every 200 ms and the filter is rebuilt when a blocked app opens or shows a window. | The visible part of each blocked top-level window is painted black in every captured frame (D3D11.1 `ClearView` on the GPU path, a CPU fill otherwise). Windows above it stay visible. The window list is read at most every 15 ms. |
| Keyboard | Remote key presses are dropped while a blocked app is frontmost. | Remote key presses are dropped while a blocked app owns the foreground window. |
| Mouse | Remote clicks and scrolls are dropped over a blocked app's window. | Same, using `WindowFromPoint`. |

Always allowed, so nothing stays stuck and the remote person can leave: key
and button releases, pointer moves, modifier keys, and Tab while Command or Alt
is held (Cmd+Tab, Alt+Tab). Other shortcuts, such as Cmd+C, are dropped in a
blocked app.

**Sanser itself** is added to the list automatically whenever it is not empty
(`com.sanser.desktop` on macOS, Sanser's program file name on Windows). The
person in control therefore cannot open Sanser on the host to remove the list,
turn off other host settings or change the account. The host engine's own
windows (voice panel, privacy screen) and the click-through annotation layer
stay as they are. The limit is 64 apps chosen by the host plus Sanser.

The person at the host computer is not affected and can use the apps normally.
Diagnostics write `SNBLOCKED apps=…` to stderr when the list is active.

## Limits

This prevents casual use; it is not a sandbox against a determined person with
full control:

- A renamed or copied program, or a portable browser, is not recognised.
- On macOS a newly opened window can be visible for up to about a quarter of a
  second before it is excluded. Its input is blocked meanwhile.
- On Windows, UWP apps hosted by `ApplicationFrameHost.exe` are not matched by
  their own name, and a rotated display uses the CPU capture path.
- Sound from a blocked app is still part of shared system audio.
- Preferences are a file in this user's app data folder. Each save in Sanser
  also stores a fingerprint of the list in the system keychain (Windows
  Credential Manager). If the file is edited with another program, sharing
  does not start and Settings → Host → Blocked apps asks the host to check the
  list and confirm it. Someone who also deletes the keychain entry is not
  caught: the next start trusts the list as on first use. Block text editors,
  terminals and Keychain Access too, or use a restricted user.
- When it matters, use view-only access or a separate, restricted user account.

## Tests

- `sanser-blocked-apps-policy-test`: identifier rules, the input gate with
  Cmd/Alt+Tab, output rectangles across monitors and black painting.
- Rust: `blocked_apps_reach_only_host_engines_and_are_validated`, tasklist parsing.
- Vitest: `BlockedApps`, preferences migration, host launch arguments.

Needs real hardware validation: Windows build and masking (not compiled in the
macOS workspace), ScreenCaptureKit exclusion timing, multi-monitor and
mixed-DPI layouts.
