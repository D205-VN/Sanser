# Legacy input acceptance after an application opens

Baseline: Sanser 2.2.21, Mac client → Windows host, Legacy Direct. The report
retained around 10:12 on 2026-10-24 has ordinary RTT near 8–15 ms, control RTT
maximum 32.8 ms and input ACK maximum 39.1 ms. Its received timing samples do
not report capture-paused state. The later samples have no new video datagrams
or rendered frames, while control and input ACK continue. This can be an idle
desktop, rejected input or a capture/output problem; it does not prove which.
The report has no app name or matching OS input-execution evidence. The user's
screenshot shows FC Online/Garena, but does not establish its privilege level or
whether a game-specific input restriction is active.

## Confirmed source defect

Legacy mouse buttons, wheel, relative motion and keyboard paths used
`mouse_event` / `keybd_event` after the normal/helper injection failed, then
treated `GetLastError() == ERROR_SUCCESS` as success. Both compatibility APIs
return void and document no success result. A stale zero error could therefore
turn a denied operation into an applied ACK and update held-state ownership.
The scan-code path could also replay a rejected key through multiple APIs.

The affected paths now trust the normal/authorized-helper injection result
instead. Rejected presses are not replayed. Rejected releases stay owned,
including failed Reset, so a later Reset can release them after recovery.
Absolute pointer placement retains its existing position-verified fallback;
the observed cursor position is distinct from acceptance of a button/key.
The change does not grant new permissions or modify broker authentication.

## Measurements added

The authenticated legacy control pong carries cumulative OS direct/privileged
acceptance and rejection counters, source/target integrity values, the last
helper failure status/error and the host input-allowed flag. Mac emits these as
`SNINPUT_OS_HEALTH`; numeric report filtering also retains ACK `applied/events`.
Older hosts omit these fields rather than inventing healthy zero measurements.
No app names, paths, typed contents or cursor coordinates are retained in this
new evidence. A zero integrity value is unknown. The last failure may predate a
later successful event; compare counter deltas and sample times.

**OS insertion is not proof that the target application acted on the event.**
Receipt/dedup ACKs likewise must not be interpreted as successful UI action.
The input-delivery ACK and retry contract is unchanged; denied presses are not
retried as new UI actions.

## Evidence and validation

- [Microsoft SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput): insertion count and UIPI restrictions; its result/error cannot definitively diagnose UIPI.
- [Microsoft mouse_event](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-mouse_event) and [keybd_event](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-keybd_event): void return, superseded APIs.
- [Parsec input troubleshooting](https://support.parsec.app/hc/en-us/articles/32381827815188-Mouse-and-Keyboard-Isn-t-Working-Correctly-When-Connected): distinguishes control permission, elevated applications and game-specific restrictions. This does not establish which applies to this screenshot.

The Windows native fixture invokes the actual legacy input methods with a
test-only OS/helper hook. It forces denial with ERROR_SUCCESS still set,
asserts one attempt per rejected event, verifies accepted input still works,
and verifies retained/recovered releases and idempotent Reset. It does not
open a real app, inject physical input or access screen/audio devices. Rust
tests verify the new numeric fields and exclusion of input/app content.

The exact failure remains to be tested on the user's Windows host: compare
Start/Explorer with the affected launcher, check whether its window keeps
changing, and correlate updated host input counters with the click. Source
repairs here are not included in the published 2.2.21 installer.
