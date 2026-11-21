# Stop remote control from the host

The person sitting at the host computer can take back control at any moment
during a session:

| Host | Hotkey |
| --- | --- |
| Windows | **Ctrl+Alt+K** |
| macOS | **Control+Command+K** |

The hotkey is a switch: press it once to turn remote control **off**, press it
again to turn it back **on**. Only the person at the host can do either. Each
press shows a short notice at the top of the host screen ("Đã tắt điều khiển
từ xa · Remote control off" / "Đã bật lại điều khiển từ xa · Remote control
on").

While remote control is off:

- The viewer's mouse movement, clicks, keyboard, scrolling, text input and
  game controller no longer reach the host. Anything the viewer was holding is
  released at the host right away, so nothing stays pressed.
- Viewers hide their pointer over the remote screen and stop sending input.
  The window title (Mac/Windows viewers) or a banner (phones) says the host
  turned remote control off.
- Viewers with chat get a message from "Sanser" each time it turns off or
  back on.
- The screen, sound, chat and pen keep working; the session stays open.
- Every new connection starts with control on.

It works whenever the host allows remote input: SNV2 hosts on Windows and
macOS, and the Windows host used by Mac viewers (there the host drops input
but the viewer is not told). It is off in view-only sessions, which have no
remote input.

While a session with remote input is open, the host engine owns this key
combination, so other apps on the host do not receive it.

## Implementation

- `native/common/control_cutoff_policy.h`: the per-session flag and the
  "releases only" rule (`sanser-control-cutoff-policy-test`).
- macOS: `platforms/macos/control_cutoff.mm`, a Carbon hot key (signature
  `SNCK`) pumped by the host loop; remote events are filtered before the
  injector. The privacy screen handler now ignores other hot keys.
- Windows: `platforms/windows/control_cutoff.cpp`, `RegisterHotKey` on a
  private message thread; every `SendInput` batch passes
  `blocked::windowsInputAllowed`, which drops it unless it only releases.
- Both hosts show the on-screen notice themselves (an `NSPanel` on macOS, a
  layered top-most window on Windows), so no notification permission is
  needed.
- Transport: the host sends an `SRC1` keepalive (1 = off) every 500 ms and
  submits one "release everything" to its input worker when control turns
  off. Viewers stop sending input (`Peer::inputAvailable`) and expose
  `Peer::remoteControlOff()`; phones get `remoteControlOff` in their status.
- Desktop shell: `SNUSER_CONTROL\trevoked` / `restored` on the engine's
  private stdout posts the chat notice (`collab::Hub::announce`).

Not yet tried on physical computers. If another app already owns the hotkey,
the engine logs `SNCONTROL hotkey=unavailable`.
