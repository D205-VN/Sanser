# SNV2 analog controller core — phase 4i1

Source increment, 2026-11-05, product 2.3.3. No release or installed-app change.

## Scope

The shared SNV2 transport now has a negotiated, authenticated single-controller
analog path and an optional Windows host backend. It carries proportional sticks,
triggers and Xbox-layout buttons without translating them into keyboard keys.
This is the **core/host increment**, not an enabled mobile analog control mode.
Mobile's existing **Tay cầm → phím** mode and `gamepad: false` registration remain
unchanged. Physical capture/UI, rumble, multiple players, and a macOS virtual
controller host are not connected by this increment.

Windows hosts check an already installed ViGEm-compatible backend: the matching
`ViGEmClient.dll` must be beside the native host executable and a compatible bus
must accept the connection. Neither is bundled, downloaded or installed here.
Loading uses an absolute executable-directory path and DLL/system-directory
dependency search, never CWD/PATH. Missing DLL/symbols/bus keeps the capability
off; attach/update failure disables it for the backend lifetime. No silent
keyboard fallback occurs. Recreate the host process after fixing its backend.

This is compatibility with the optional backend already used in Legacy, not a
recommendation to adopt it as a new long-term driver dependency: the upstream
[ViGEmBus](https://github.com/nefarius/ViGEmBus) and
[client](https://github.com/nefarius/ViGEmClient) repositories are archived.
The ABI uses the public [XUSB report layout](https://github.com/nefarius/ViGEmClient/blob/master/include/ViGEm/Common.h)
and [client API](https://github.com/nefarius/ViGEmClient/blob/master/include/ViGEm/Client.h).
No third-party driver implementation was copied into Sanser.

Phase 4i2 subsequently adds [negotiated mobile analog capture/UI](mobile-analog-controller.md).
The scope statements above describe the original core increment; mobile now
has a separate opt-in analog action alongside its keyboard profile. Device/
driver validation, desktop-native capture and rumble remain outstanding.

## Wire and capture contract

Hosts periodically advertise authenticated `SGC1 + enabled-byte` keepalives.
Only control-enabled, non-sharing hosts with an available backend advertise 1.
The client ignores older reordered capability advertisements and expires the
capability after three seconds without an update. Old hosts never enable it.
`Peer::gamepad(state, expectedRemoteGeneration)` checks capability, live input/
capture permission, connection state and the caller's remote-generation fence.
Ordinary `Peer::input` cannot bypass the negotiated gamepad path.

The existing SNV2 `Gamepad` type carries an 18-byte `SGP1` payload: connected,
reserved-zero, button mask, two 8-bit triggers and four signed 16-bit sticks
(network byte order; Y upwards). One controller is supported. Guide/reserved
buttons, malformed lengths/versions and nonneutral disconnected payloads are
rejected. No floating-point NaN or arbitrary key sequence is admitted.

States use stream 0 with independent latest-state sequence checks, not reliable
keyboard stream 2. The sender has a dedicated one-packet slot independent of
mouse motion; mouse/gamepad slots alternate when both are pending. Reliable
keys/buttons stay ahead of these slots, then voice/audio and video. The gamepad
slot expires unsent states older than 100 ms and never accumulates old analog
positions or consumes the reliable ACK window. A future capture caller must
send a full snapshot at least every 50 ms, including while a stick is held.
Very short button transitions can be lost because this is snapshot transport.

Every snapshot's authenticated header includes the latest reliable input
sequence as a fence. Hosts accept it only after that same sequence has been
delivered. A pre-reset neutral packet arriving late cannot rearm a replacement
capture. This also means snapshots can be skipped while reliable keys/reset
are being repaired; the caller continues publishing fresh state.

## Host lifetime

The existing OS-input worker owns gamepad dispatch and release; receive/ACK/
decode threads never wait for driver injection. At most one pending gamepad
snapshot survives in its bounded queue. Its receipt timestamp is preserved,
so stale queued work cannot renew the controller lease.

Input first arms from a centered sample (no buttons, triggers ≤8, sticks within
±4096). If snapshots stop for 250 ms, the idle worker emits a disconnected neutral
state and requires neutral again. Reset, capture pause/resume, peer generation
change and stop disarm it through the existing input-reset fence. The worker
sleeps until new input or its active lease deadline; an idle non-gamepad session
does not gain periodic polling. The deadline cannot preempt an OS call already
in progress. Backend failure withdraws capability and performs best-effort
neutral/detach without replaying a rejected press or terminating media.

## Validation and remaining work

Portable tests cover signed-axis endpoints, wire rejection, neutral arming,
stale/expired state, an idle-worker disconnect watchdog, 1,000 replaced snapshots,
independent mouse state and fake driver attach/update failure. The same tests
run with ASan+UBSan and cross-build as a standalone Windows executable registered
in the existing Windows input CI job.

Authenticated UDP loopback tests cover analog values, old/denied/sharing hosts,
forged input on denied hosts, stale caller generations, a delayed pre-reset
neutral sample, a lost disconnect, capture pause, capability withdrawal and
continued video delivery. Input engine, input recovery, reliable text/shortcuts
and transport suites provide regression coverage.

No Windows driver/game runtime, physical controller/mobile bridge or real
latency benchmark ran on this Mac. LLVM-MinGW's older MF headers omit
`MF_LOW_LATENCY`; integrated Windows backend syntax checking needs a declaration-
only shim for that existing symbol. This is not a full Windows/MSVC build.
Next: negotiated mobile/native capture UI and physical-device testing, then
rumble and backend strategy. Roadmap phase 5 voice/performance A/B remains separate.

## Desktop capture follow-up

[Phase 4i3](native-desktop-controller.md) adds opt-in GameController/XInput
capture on SNV2 native viewers. Its shared sender is also used by mobile.
Desktop entry uses bounded `releaseForController` independent of text capability;
focus/reset/generation and stale-poll guards stop capture without automatic
restart. The original core-only scope statements above are historical. Real
controller/driver validation, rumble and macOS virtual hosting remain.
