# Application launch freeze: capture, input and transport

Investigation date: 2026-10-23. Source baseline: Sanser 2.2.20, commit
569762ac23a3db69a10a3e8ce48e3730057423a9. These follow-up fixes are source
changes; they are not included in the published 2.2.20 installer.

## Evidence from retained Sanser reports

Two recent bounded local reports identify macOS client → Windows host, legacy
Direct. They contain numeric metrics only; they do not identify the launched
application or prove which moment corresponds to the user's reproduction.

| Measurement | Ultra session | Subsequent balanced session |
| --- | ---: | ---: |
| Control app RTT P50 / maximum | 10.1 / 7087.8 ms | 8.0 / 53.2 ms |
| Input ACK RTT maximum | 5812.2 ms | 65.6 ms |
| Raw UDP echo RTT P50 / maximum | 8.2 / 2362.0 ms | 9.6 / 26.0 ms |
| Host input processing maximum | 28.9 ms | 26.2 ms |
| Host control work maximum | 0.1 ms | 0.1 ms |
| Mac UDP receive queue maximum | 26.1 ms | 0.3 ms |

The Ultra report includes sequence/fragment losses and a UDP echo spike. Many
later echoes return to 7–11 ms. These are not simultaneous, matched control and
echo probes: the data cannot rule out transient kernel/NIC/router queues, host
scheduling stalls, packet loss or control-path-specific delay. It does not
establish that Internet distance, shared socket locking, UAC or video decoding
alone caused the freeze. Capture-paused values in received host timing samples
are zero; a missing interval cannot establish that capture never paused.
An idle desktop's low video count is not proof of a frozen capture pipeline.

## REA and primary-source comparison

REA **6.5.0** was restored in a temporary directory because the previous temporary
installation had lost package files. It was not added to Sanser dependencies or
agent configuration. `inspect-artifact` examined installed Parsec 150.101.1 and
AnyDesk 9.6.5; `inspect-macho` examined Parsec's shipped core and AnyDesk's
executable. Neither application was launched. Complete evidence is under
`/private/tmp/sanser-app-launch-research-2026-10-23/`; compact identifiers/hashes
are in [the evidence record](app-launch-freeze-evidence.json).

These are static Mac artifacts, not Windows binaries or a runtime UAC trace.
Imports/artifact inventory do not prove competitor recovery behavior. Windows
behavior below comes from pinned open-source code and official product/API
documentation, not an invented proprietary decompilation.

| System | Verified behavior | Implication for Sanser |
| --- | --- | --- |
| [Sunshine capture](https://github.com/LizardByte/Sunshine/blob/d0b4a88ef727f8dabf80cc970a14739b55c885de/src/platform/windows/display_base.cpp) | Synchronizes its capture thread's desktop, treats access-lost/DXGI-access-denied/abandoned synchronization as reinitialization, and separates idle timeouts | Desktop transitions should invalidate capture resources, not necessarily the session |
| [RustDesk video service](https://github.com/rustdesk/rustdesk/blob/25d27422733920926cff0c27f8e37f8fdbd26288/src/server/video_service.rs) | Limits DXGI restart attempts within a window and has GDI fallback | Recover the capture backend deliberately and retain evidence before selecting a slower fallback |
| [Parsec Windows documentation](https://support.parsec.app/hc/en-us/articles/32381199341716-Parsec-App-for-Windows) | Describes its service as enabling elevated capture/input | A normal user process cannot assume permission to control every administrator window |
| [AnyDesk elevation documentation](https://support.anydesk.com/administrative-privileges-and-elevation-uac) | Distinguishes installed service/elevated operation from ordinary portable operation | Diagnose capture access and injection permission separately |
| [Microsoft AcquireNextFrame](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe) / [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput) | Desktop/full-screen/mode transitions invalidate duplication; UIPI restricts input to equal/lower integrity | A drawn window and an input receipt ACK do not prove input was executed |

## Confirmed code defects repaired

1. Windows capture recognized `E_ACCESSDENIED` but missed the distinct
   `DXGI_ERROR_ACCESS_DENIED` and `WAIT_ABANDONED`. Errors from `Map`/texture/query
   operations after acquisition also escaped the existing recovery path.
   Typed capture-operation errors now unwind frame/map cleanup before entering
   the existing throttled recreate path. The same DesktopDuplicator serves legacy,
   SNV2 and the already-authorized capture helper. Idle timeouts do not trigger
   recreation; invalid API use, out-of-memory and GPU command bugs stay visible.
   Existing generation changes retain fresh-frame/keyframe recovery and device
   changes still recreate the encoder. No new capture permission is granted.
2. The SNV2 Windows injector changed its held-key/button sets before inspecting
   SendInput's result. A rejected release erased ownership of a key/button the OS
   could still hold. State now changes only after accepted injection. Reset can
   retry a failed release after desktop recovery, and never replays a press.
   Rejected injection is logged separately from capture failure. This does not
   itself establish or eliminate the legacy RTT stall in the retained report.
3. The SNV2 Windows injector copied the display rectangle only at construction.
   A capture-generation change after a mode/full-screen transition updated video
   but retained old mouse coordinates. The capture thread now publishes the new
   rectangle under a short mutex; the input thread reads a snapshot. Zero-sized
   unavailable displays cannot underflow coordinate calculation. Legacy already
   refreshed its input bounds and is not changed for this issue.

New bounded numeric report sources retain capture error codes and input
rejections. They discard app names, input contents, paths and stage strings.
They provide evidence of a failure, not a claim that an ACK means OS execution.

## Validation / physical acceptance

A device-free native fault-injection test covers desktop/GPU errors, resource
cleanup before recreation, successful subsequent capture, idle polls and fatal
errors that must propagate. Windows SDK constants are checked on Windows.
The native Windows input fixture simulates denied down/up and recovery without
opening real apps, injecting physical input or invoking UAC. It also verifies
pointer mapping after resolution changes and zero-sized display transitions.
Rust tests cover
numeric evidence filtering. Windows CI must validate the platform integration.

The first Windows CI build passed capture recovery and input tests, but its
prerecorded codec fixture failed after cold Media Foundation initialization took
964 ms: its finite source overfilled the intentionally bounded live queue and
could not respond to the resulting IDR request. The fixture now starts its frame
clock after the first access unit is consumed (without waiting for output).
Continuous decoding, final-picture and frame-count assertions remain; the
low-delay fixture retains its strict one-input/one-output timeout. Synthetic
DecodeWorker tests separately cover cold queue retention, overflow and expiry.
Production queue/deadline policies are unchanged by this test correction.

Remaining physical reproduction: identify the direction and launched app;
compare an ordinary window, its elevated variant and full-screen/mode switch
separately. Check local host responsiveness and whether video/audio/pong/input
ACK continue, then export reports from both computers. Match event times before
attributing the seconds-long gap to network queues, capture, injection or decoder.
No benchmark of competing apps, live traffic capture, Windows policy/service
change, installation on the user's machine or new release was performed here.
