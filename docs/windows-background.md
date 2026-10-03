# Windows background operation

Closing the main Sanser window with X or Alt+F4 hides it in the Windows system
tray. The webview remains alive, so signing in, Host online status, incoming
requests and active native sessions are preserved. Closing the native remote
desktop viewer still ends that viewer normally.

- Left-click the Sanser tray icon, or choose **Open Sanser**, to restore the window.
- Right-click the icon and choose **Quit Sanser** to exit and stop native engines.
- Turn Host online before hiding the window. Hiding it does not enable Host or
  grant access to any additional computers. Existing trust/approval rules apply.
- Windows may place the icon under the taskbar's hidden-icons arrow.

This is a running application, not a Windows service: the computer must remain
awake and the Windows user must stay signed in. It does not install startup tasks
or start Sanser automatically after reboot. macOS window behavior is unchanged.

Host heartbeat and request polling currently run in the webview. Windows startup
adds WebView2's `--disable-background-timer-throttling` argument to preserve those
timers while hidden. Native video/input and relay loops remain independent of
window visibility. This should be checked on the installed Windows WebView2
runtime, including after more than five minutes in the tray.

## Windows acceptance check

1. Sign in, bring Host online, then close the main window. Check that the tray icon
   remains, the taskbar window disappears and a connected viewer stays connected.
2. Leave it hidden for at least ten minutes, then connect from another computer.
   A trusted device with auto-accept enabled should connect normally. Untrusted
   devices must still wait for approval; reopen the host to approve them.
3. Reopen from both left-click and the menu; confirm the same account/session.
4. Quit from the tray and confirm Sanser and its native children have stopped.
   Also verify an updater restart exits instead of hiding the old app.

References: [Tauri system tray](https://v2.tauri.app/learn/system-tray/),
[Microsoft WebView2 browser flags](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/webview-features-flags).
