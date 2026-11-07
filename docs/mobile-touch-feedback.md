# Mobile touch feedback — phase 4b

Source increment, 2026-11-05, Sanser 2.3.3. This does not release or install an app.

## User behavior

**Cài đặt → Rung khi thao tác** enables local feedback for remote touch controls.
The same setting is accessible from **Cài đặt phiên** in the floating panel.
It defaults off, including when restoring preferences from older versions, and
persists on this phone. Changing it applies to the current session immediately.

| Interaction | Requested feedback |
| --- | --- |
| Valid direct tap or touchpad tap | Selection click |
| Long press/right-click or start of a direct drag | Light impact |
| Touch game button press | Selection click |
| Game stick leaves its center deadzone | Selection click |
| Pointer motion, scrolling, pinch/zoom, held/repeated keys, button release/cancel | None |
| View-only, unavailable session, covered route or background app | None from remote controls |

This is feedback for the **local gesture**, not confirmation that the host
accepted a click. Physical keyboard input and the local text composer remain
independent. Adaptive Settings controls can retain their normal OS feedback;
the preference controls Sanser's explicit remote-touch feedback.

## Scheduling

`MobileHaptics` uses Flutter's existing platform feedback channel; there is no
new package, native media path or network message. Remote input is queued
independently, without awaiting a haptics reply. Video/audio work stays native.

One feedback request may be in flight at a time, with at least 75 ms between
accepted requests using a monotonic clock. Extra requests are dropped, not
stored for later. A stalled platform channel therefore suppresses subsequent
feedback rather than building a queue or blocking click delivery. Unsupported
hardware/channel errors are ignored. Backgrounding, closing the session or
disabling the option never replays skipped feedback. An already-dispatched
physical pulse is not cancelled by the app.

Only Android/iOS request this explicit feedback. Strength and availability
depend on the actual device and system settings; no fixed motor behavior is
promised for every phone or tablet.

## Validation

`apps/mobile/test/haptics_test.dart` checks preference migration/serialization,
live readiness/opt-in/lifecycle gates, mobile-only emission, throttling and
backpressure, missing platform support, gesture selection and read-only input.
A deliberately unresolved platform reply verifies that a click still sends
both down and up on the original session. The settings test also toggles and
restores the option through the UI and existing preference store.

All 83 Flutter tests pass and `flutter analyze` reports no issues locally.
An isolated unsigned iOS Release also builds successfully (23.5 MB Runner.app);
no installation or signing configuration change was made.
Tests mock the haptics channel; they do not verify motor sensation, battery use,
WAN reliability or a measured reduction in streaming latency. USB/Bluetooth
keyboards, actual haptics and target applications still need physical-device
validation. [Adaptive tablet layout](mobile-tablet-layout.md) is now implemented
in source, as is [bounded network-change recovery](mobile-network-recovery.md).
[Physical controller → keyboard](mobile-controller-keys.md) is a separate source
profile; virtual gamepad and controller rumble remain unimplemented.
