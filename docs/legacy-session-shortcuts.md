# Legacy Mac → Windows host shortcuts — phase 4h

Source increment, 2026-11-05, Sanser 2.3.3. No release or installed-app change.

## Use and compatibility

The Legacy Mac viewer now uses the same native **Shortcuts** title-bar button
and modeless action panel as the [SNV2 desktop viewer](native-session-shortcuts.md).
Select **Windows host**, select an action, then **Send shortcut**. The Mac-host
segment is disabled on this Windows-only engine. Group/view-only viewers do not
receive an input panel. No engine-selection preference is changed.

Both the Legacy client and Windows host need this implementation. The opener
stays disabled until the host advertises `hostShortcuts: true` in an authenticated
control-pong, with authenticated control, packet authentication and input-batch
support active. HELLO metadata or an unsigned pong cannot enable it. Older hosts
continue with ordinary input; they do not receive unsupported shortcut requests.

Opening releases physical keyboard/mouse/gamepad input and clears its queued
work before a reliable panel-entry reset. Physical sources stay released after
closing; **left-click the stream** to resume. Relative pointer grab resumes at
that click. Losing focus, capability, route/authentication or the panel epoch
cancels its unsent/retry work. An already transmitted request cannot be recalled.
Control pings, repair and media workers continue while the panel is open.

## Fixed operation and delivery

Legacy's physical keyboard intentionally translates Mac Command to Windows
Ctrl, so sending synthetic Mac events would turn Win+D into Ctrl+D. Instead,
the panel sends a fixed named operation, for example:

```json
{"type":"host-shortcut","action":"desktop"}
```

It is one event in the existing authenticated reliable input-batch, not a new
socket or shell command. Only the shared Windows shortcut profile is accepted;
no peer-provided key arrays, process names or scripts are supported. The host
checks authenticated channel state and live remote-input permission, then uses
its Windows HID/scan-code mapping, including the real Win key. Copy/Paste uses
the host clipboard, without reading the viewer clipboard.

Each reset/action waits for its predecessor's ACK. Retries retain the batch
sequence and use the existing host deduplication. A rejected/stale ACK or an
exhausted retry deadline cancels dependent actions instead of advancing a Paste
after an unconfirmed Copy. Admission is bounded to 128 pending/queued operations;
a full queue refuses the new action. Closing/resuming/rebinding invalidates old
UI generations. This adds no video scheduling or latency-policy changes.

Before dispatch, the host releases tracked remote state and verifies neutral
keys/buttons. Fixed keys press in order and release in reverse. A partial OS
failure receives best-effort reverse releases, with no replay of the key-down;
failed releases remain tracked for existing reset/watchdog recovery. Legacy's
reset is synchronous state release, distinct from SNV2's InputWorker queue fence.
Target applications, keyboard layouts, UAC and host OS permissions still decide
whether an operation works. The UI reports queued, not application success.

## Verification and limits

Targeted checks cover authenticated UDP loopback with missing ACK/retry,
predecessor ordering, rejected ACK/deadline cancellation, bounded admission,
physical/gamepad isolation, stale epochs, view-only/older hosts and signed
capability revocation. Shared AppKit tests verify the disabled Mac profile and
single close callback. Portable host tests verify all fixed mappings, permission
loss, failed neutral reset and partial reverse cleanup; they also run with
ASan+UBSan. The existing Legacy latency regression remains included.

The Mac client builds and links. Windows host source with mock input-dispatch
tests syntax-checks using LLVM-MinGW, and standalone Windows shortcut/IME tests
cross-build. Windows mock-dispatch runtime cases are registered in the existing
host input-dispatch test but **have not run on this Mac**. AppKit tests use fake
callbacks, not a remote Windows application.

Real Windows/MSVC execution, normal/elevated target applications, fullscreen,
non-US layouts and real two-machine behavior remain to validate. This work does
not establish lower RTT, complete virtual Xbox/analog/rumble support or complete
phase 5 voice/performance A/B measurements.
