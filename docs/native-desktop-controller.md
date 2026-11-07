# Native desktop analog controller — phase 4i3

Source increment, 2026-11-06, Sanser 2.3.3. No release, installed-app replacement
or virtual-driver installation.

## Use and scope

In an **SNV2 control viewer**, enable **Controller** in the macOS title bar, or
**Analog controller** in the Windows window's system menu / **Ctrl+Alt+G**.
Connect one controller through the operating system. Center sticks and release
buttons/triggers to arm. **Start+Select** stops locally. The normal input-release
shortcut and switching applications also stop capture; returning never restarts it.

Availability requires authenticated `SGC1` from a supporting host. Currently the
production host provider is the [optional Windows virtual-controller backend](native-gamepad-core.md).
Unsupported/old/macOS hosts and view-only/group sessions cannot enable this mode.
This does not change the Legacy Mac → Windows controller path or select a new
transport automatically. Both ends need compatible source/builds.

Mac uses an already connected GameController extended gamepad. Windows uses one
XInput controller; its DLL is loaded only from System32. This does not provide
generic DirectInput/vendor support, pairing, rumble, gyro, multiple players or a
macOS virtual-controller host. No new third-party input driver is installed.

## Capture and ownership

`native/desktop/controller_capture.h` now contains the physical button/axis
mapper and sender previously used by mobile. Mobile retains a compatibility
header and its existing C ABI. Both use one latest snapshot, neutral arming,
an 8 ms minimum send interval for new state and a 40 ms held-state heartbeat.
Axes retain signed magnitude and triggers remain proportional. Guide is not sent.

Desktop polls its native source at most every 16 ms on the viewer UI thread.
The sender thread never calls AppKit/Win32 focus APIs. `ControllerSession` reads
atomic focus, generation and host availability. A 200 ms poll-freshness limit
stops replay of stale held input if the UI stalls; the host's independent
250 ms lease remains the fallback when a disconnect packet is lost. This cannot
preempt a blocked OS/driver call or recall packets already sent.

On entry, `Peer::releaseForController(epoch)` admits one reliable Reset through
the bounded input lane. It requires analog capability rather than unrelated
Unicode capability. A full reliable window rejects entry without terminating
media or reserving a partial sequence. Gamepad snapshots carry the resulting
reliable sequence fence, so an old neutral packet cannot rearm the new capture.

Opening text/shortcut controls, Reset, focus/application loss, host capability
withdrawal, generation change and viewer close stop capture. Windows also stops
on native menu/resize entry. Device replacement must center again. Native UI
callbacks/timers are detached before the shared model/Peer is destroyed.
Status describes local connection/arming, not confirmed game acceptance. Short
button pulses may be missed by polling/snapshot delivery; no reliable edge or
rumble protocol was added.

## Verification

- Mac native client builds; nine focused desktop suites pass: controller model,
  AppKit controller, analog transport/core, text, shortcuts, AppKit IME and input
  engine. Reset-hook and full reliable-window cases have dedicated regression checks.
- The shared controller model passes ASan+UBSan with warnings treated as errors.
- Two native mobile suites pass after extraction: physical analog sender and
  C ABI interoperability, including video/input/teardown regression coverage.
- Standalone Win32 controller/menu and model test executables cross-build on Mac,
  alongside existing Windows IME/shortcut tests. Windows backend syntax-checks
  with the existing declaration-only workaround for old MinGW's missing
  `MF_LOW_LATENCY` declaration. This is **not** a full Windows backend link/runtime test.

No physical controller, Windows virtual driver/game or end-to-end latency was
measured. Real USB/Bluetooth controllers, Windows/MSVC execution, adverse-network
behavior and phase 5 voice/performance A/B remain required.

API references: [Apple GCController](https://developer.apple.com/documentation/gamecontroller/gccontroller),
[Microsoft XInputGetState](https://learn.microsoft.com/en-us/windows/win32/api/xinput/nf-xinput-xinputgetstate).
