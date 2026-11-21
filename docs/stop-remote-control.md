# Stop remote control from the host

The person sitting at the host computer can take back control at any moment
during a session:

| Host | Hotkey |
| --- | --- |
| Windows | **Ctrl+Alt+K** |
| macOS | **Control+Command+K** |

After the hotkey:

- The viewer's mouse, keyboard, scrolling, text input and game controller no
  longer reach the host. Key and button releases still pass once, so nothing
  the viewer was holding stays pressed.
- The screen, sound, chat and pen keep working; the session stays open.
- It lasts until the session ends and cannot be undone from either computer.
  The next connection starts with control again.
- Viewers with chat get a message from "Sanser" saying the host turned off
  remote control. The host engine also writes `SNCONTROL remote=revoked` to
  its diagnostics.

It works whenever the host allows remote input: SNV2 hosts on Windows and
macOS, and the Windows host used by Mac viewers (no chat message there). It is
off in view-only sessions, which have no remote input.

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
- Desktop shell: `SNUSER_CONTROL\trevoked` on the engine's private stdout
  posts the chat notice (`collab::Hub::announce`).

Not yet tried on physical computers. If another app already owns the hotkey,
the engine logs `SNCONTROL hotkey=unavailable`.
