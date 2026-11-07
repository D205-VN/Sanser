# Text clipboard and desktop Unicode input

UI roadmap phase **3b**, source implementation dated **2026-11-04**. Requires
updated native engines and an **SNV2 control session**; legacy media sessions and
view-only sharing do not expose remote clipboard actions. This work is not a
published release.

## Using it

1. On the desktop host, enable **Settings → Host → Text clipboard**, leave remote
   input enabled, then reconnect. The clipboard setting defaults to off and is
   applied when the host engine starts.
2. Desktop controller: open **Active session → Text and clipboard**. **Send text**
   types a committed Unicode edit into the host's focused field. **Read host
   clipboard** fills the editor for review. **Write host clipboard** updates the
   host's text clipboard; it does not type/paste it into an application.
3. **Paste locally / Copy locally** use the controller's native OS clipboard.
   They run only on an explicit button press. Receiving remote text does not
   automatically replace the local clipboard.
4. Android/iOS: open **Clipboard** from the floating session controls. Use the
   receive/send and local paste/copy buttons in the sheet. The remote feature
   stays disabled until host permission is confirmed. Mobile Unicode typing
   remains available separately under **Bàn phím**.

Committed typing keeps the existing 256 UTF-16-unit limit. Clipboard text is at
most **8 KiB of UTF-8**, supports CRLF/LF/tab, Vietnamese, combining marks and
emoji, and rejects NUL, unsupported controls and malformed Unicode. It transfers
plain text only, without images, file clipboard formats or remote IME composition.
Applications that ignore Unicode text events still need the physical-key path.

## Transport and scheduling

Clipboard uses authenticated SNV2 `Clipboard` packets on stream 0 with a separate
`SUD1` application-message envelope. It never consumes reliable input sequence
numbers. Each fragment carries a message ID, index, count and total size. The
bounded sender uses four message slots and a four-fragment window, selective
fragment ACKs, retries based on measured RTT, a ten-second sender lifetime, and
deduplication of delivered messages. Protocol messages are capped at 32 KiB;
clipboard RPC bodies have the stricter 8 KiB text limit.

The TX scheduler gives user data a dedicated 16 KiB lane after input/control,
voice/audio and video. It coalesces retries that are already queued. A separate
credit bucket permits at most **2% of the video wire budget, capped at 64 KiB/s**,
and also charges the shared wire budget. Data starvation is bounded without
putting clipboard traffic into the input lane. Full data queues and clipboard
errors do not terminate the video session. Expired queued data is discarded.

Receivers bound partial-message memory and reject inconsistent fragment metadata.
Data state and outstanding clipboard requests reset on authenticated generation
changes. Pending mobile callbacks complete when the peer stops, so Swift/JNI
completion contexts are released. A timed-out write can have reached the OS;
there is no automatic application-level retry of a clipboard write.

`SCB1` is the remote clipboard capability and RPC body tag. Requests contain an
operation and request ID. Host permission, remote input permission, sharing mode
and capture pause are checked at the host before an OS operation. A forged
capability at the controller does not grant access. Clipboard OS jobs have a
five-second local validity guard and capture no raw `Peer*` for later callbacks.
Only explicit local IPC commands can use the controller's OS clipboard; network
requests received by a controller cannot invoke its clipboard provider.

## OS and private IPC

macOS schedules pasteboard work on the main queue, with bounded UTF-8 conversion
and cancellation checks. Windows uses a short-lived worker and a message-only
clipboard owner window, materializes `CF_UNICODETEXT`, and closes/unlocks handles
on all paths. A locked clipboard reports busy rather than blocking receive or
capture with a long retry loop. See [Microsoft OpenClipboard ownership rules](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-openclipboard)
and [Apple NSPasteboard](https://developer.apple.com/documentation/appkit/nspasteboard).

Desktop payloads cross stdin/stdout as bounded private records. A single bounded
stdin writer preserves command ordering and keeps pipe I/O outside the global
engine-state mutex. Replies identify the individual request and actual viewer
session. The stdout reader strips **all** `SNUSER_` records before diagnostic
tails, metrics or debug logs, including malformed, oversized and interleaved
records. Neither clipboard contents nor typed text are diagnostic samples.

Desktop and mobile editors block duplicate submissions and ignore replies from
closed/previous sessions. Clipboard receive fills a review field; only the
explicit local copy action modifies the controller's clipboard. No periodic
clipboard content polling, auto-paste or local clipboard upload is implemented.

## Validation and remaining work

Native tests use authenticated loopback peers and injected lost/duplicated
datagrams. They check reassembly, ACK loss, exact-once provider invocation,
permission/read-only enforcement despite a forged capability, independent input/
video progress, local-versus-remote routing and completion of pending C ABI
requests during close. Fake clipboard providers avoid reading or overwriting
the developer's real clipboard.

Rust tests check private-data removal, bounded requests, capability gates,
session isolation and graceful child shutdown through the command writer.
Desktop/Flutter tests cover manual review/copy, byte limits, pending operations
and stale/disposed UI. Swift is checked by an unsigned iOS build and JNI by an
official-header syntax check. Physical clipboard behavior and Windows/Android
full platform builds still require validation on their respective devices/SDKs.

Phase 3c now builds [consented desktop file transfer](file-transfer.md) on `SUD1`:
offer/accept/cancel, bounded streaming and integrity checks without overwrites.
Mobile now uses the same Rust worker with native document pick/export. These
source changes have not been released yet.
Full remote IME composition also remains separate from the committed-text editor.
