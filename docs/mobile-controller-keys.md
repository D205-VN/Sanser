# Physical mobile controller → keyboard — phase 4e1

Source increment, 2026-11-05, Sanser 2.3.3. Not a release or device validation.

## Scope and use

At phase 4e1 the shared SNV2 host had no virtual-gamepad injector or controller-state
wire contract. This increment implements **physical USB/Bluetooth controller →
keyboard**, not Xbox/XInput emulation, proportional analog input, mouse aiming
or rumble. Server registration deliberately retains `gamepad: false`. The
separate local mobile capability is `controllerKeys`.

Connect a supported controller through the OS first. In a streaming control
session open the floating panel and select **Tay cầm → phím**, then tap the video.
Status distinguishes waiting, connected-but-unarmed and ready. Return sticks,
triggers and buttons to neutral before input is accepted. On Android availability
is not a neutral hardware sample: move/release a control once if still unarmed.

The mode starts off in every session and is mutually exclusive with external
keyboard capture. View-only cannot enable it. Local editors/settings, loss of
focus/background, teardown and network recovery release ownership and stop
capture. Touch controls have separate key owners. Hold **Start+Select** together
to release capture; the chord is not sent as Enter+Tab. Individual Start/Select
retain their mappings.

## Fixed profile

| Physical control | Remote keyboard |
| --- | --- |
| Left stick | W/A/S/D |
| Right stick / D-pad | Arrow keys |
| A / B / X / Y | Space / Esc / E / R |
| Left / right shoulder | Q / F |
| Left / right trigger | Shift / Ctrl |
| Left / right thumbstick press | Shift / Ctrl |
| Start / Select | Enter / Tab |

Directions enter at magnitude 0.35 and release below 0.20; triggers enter at
0.55 and release below 0.35. Hysteresis prevents repeated transitions near a
single threshold. These are digital keys; a half-tilted stick is not half-speed
analog movement. The target app must accept/configure keyboard bindings.
OS-reserved buttons and unsupported controller layouts remain local.

## Platform and scheduling contract

iOS receives GCExtendedGamepad changes and declares optional ExtendedGamepad
support in Info.plist. Android handles app-local gamepad/joystick key/motion
events and InputManager attach/remove callbacks. Neither implementation scans/
pairs Bluetooth or installs drivers. One eligible controller is used at a time;
device IDs/names stay native. A disconnect/reconfiguration emits neutral before
choosing a replacement, which must arm from neutral again.

Buttons flush immediately. Analog-only movement has one latest-state 16 ms
dispatch slot rather than a queue of old samples. The bridge carries bounded
control values and capture/session counters; media stays native. Mapped HID
transitions reuse reliable input without a new wire protocol or controller driver.

Capture requires the matching session/media attempt and a rendered control
stream. Each activation has a new capture ID; stale snapshots/disable commands
cannot affect a replacement. Session/controller consumers share one platform
status subscription. The session ignores local motion; controller UI updates
only for connected/armed/error changes, not every axis sample.

Independent owners prevent a trigger, D-pad or touch control releasing another
source's key. Invalid/disconnected/expired state is neutralized. A closed status
stream refuses blind reactivation and asks to restart the app. Queued input also
checks the original media revision during network recovery.

## Validation and remaining work

Twelve controller tests cover arming, hysteresis, exact mapping, button edges,
ownership, malformed/read-only input, session/attempt/capture fences, local
editor focus, Start+Select, native pause, delayed replies, errors/closed streams,
UI notification cost and a shared native-event subscription. An additional
session test verifies 100 motion snapshots cause no workspace notification or
telemetry mutation. The full Flutter suite passes 115 tests; analyzer is clean.

An isolated unsigned iOS Release builds with GameController (23.6 MB Runner.app).
All Kotlin sources compile against Android 36/Flutter/JVM 17; this is not an APK
or device result. Its existing Media.kt elvis warning remains. No personal signing
settings or protected desktop binary changed.

Real USB/Bluetooth Xbox/PlayStation/MFi/vendor controllers, button naming,
disconnect/reconnect, OS overlays, target apps and game bindings still need
device validation. Custom profiles, player selection, mouse aiming, virtual
gamepad-state transport, Windows/macOS virtual-controller backends and rumble
remain separate. This does not complete the native gamepad milestone.

Phase 4i1 subsequently adds the [shared analog core and optional Windows host
backend](native-gamepad-core.md). Mobile has not switched to that contract;
its keyboard profile and `gamepad: false` registration remain unchanged.
Phase 4i2 adds a separate [mobile analog mode](mobile-analog-controller.md) gated
by the live host capability. The keyboard profile itself still sends only keys.

Primary references: [Apple GameController](https://developer.apple.com/documentation/gamecontroller),
[Apple handler queues](https://developer.apple.com/documentation/gamecontroller/gcdevice/handlerqueue),
[Apple configuration](https://developer.apple.com/documentation/xcode/configuring-game-controllers),
[Android controller events](https://developer.android.com/games/sdk/game-controller/controller-input).
