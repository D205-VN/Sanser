# Consented file transfer — desktop and mobile

Source implementation dated **2026-11-04**, not a published release. This adds
bidirectional file transfer to updated **SNV2 control sessions** (Mac / Windows
desktop and Android / iOS controllers). Legacy
sessions, view-only sharing, and old engines do not advertise this capability.

## Use

The controller uses **Active session → File transfer**; the host uses
**Host → File transfer**, below the current-client card. Choose a file and click
**Offer file**. Only the name and size are sent until the other person clicks
**Accept and save**. A file invitation banner appears throughout the signed-in
desktop workspace and links to the appropriate panel. Each direction has one
transfer slot. Files are limited to **25 MiB** in this first implementation.

On mobile, open **Truyền file** in the floating controls, **Chọn file**, then
**Mời nhận file**. The picker copies a user-selected regular file into a private
workspace, with a streaming 25 MiB cap even when a provider reports no size.
Only its opaque source UUID and name cross Flutter; file contents stay native.
Mobile and desktop both use `crates/sanser-file-transfer` for consent, offsets,
timeouts, incremental SHA-256, private storage and publication. Mobile's worker
automatically streams source chunks after acceptance, without a Dart byte pump.

Incoming mobile offers show a notification over the remote view. **Chấp nhận**
authorizes receipt into Sanser's private storage. **Tệp đã nhận** (also under
Settings) lists verified files independently of the active session. **Lưu** opens
iOS's document export picker or Android's `ACTION_CREATE_DOCUMENT`; cancellation
does not delete the private original. No broad Android storage permission is
requested. The initial inbox is bounded to 256 items/10,000 scanned entries.

Selecting/exporting through the OS does not by itself end the remote session.
Held input is released, and a background document-UI lease lasts at most two
minutes; normal backgrounding still disconnects. Session/generation changes
invalidate pending picker results. Completed received files survive disconnect;
owned source copies and unfinished transfers are cleaned up when the engine ends.

The receiving computer saves into its OS Downloads directory under `Sanser`.
The final filename has a random UUID prefix and the offered basename. Paths,
unsupported/control characters, deceptive bidi controls, oversized names and
invalid offsets are rejected. Files are never opened automatically. There is no
remote filesystem browser, directory transfer, automatic overwrite or resume.

Both sides can cancel. Leaving the sending panel cancels its upload because its
browser-selected file handle belongs to that panel. A receiver that already
accepted may leave the panel while its worker finishes. Closing the engine,
authenticated generation changes and inactivity cancel unfinished transfers and
remove their partial files. A completed file remains in Downloads.

## Protocol and ownership

`SFT1` is an optional Keepalive capability. It is announced only by an opted-in
control engine. Both native peers enforce local input permission and
exclude sharing mode; a forged capability does not grant access. The environment
flag `SANSER_FILE_TRANSFER=1` is set only by the desktop shell for SNV2 control
engines, so older engines ignore it without receiving an unknown CLI argument.
Mobile enables it through the C ABI before starting its peer, after creating
the Rust worker. It cannot be enabled mid-session.

File messages use the existing authenticated/encrypted SNV2 `Clipboard` packet
type and `SUD1` user-message lane, independently of clipboard RPCs and reliable
input sequences. A file message is `SFT1 | operation:u8 | UUID:16 | body`:

| Operation | Body | Effect |
| --- | --- | --- |
| Offer (1) | size:u64 BE, UTF-8 basename | Invitation only; no disk work |
| Accept (2) | empty | Begin the accepted transfer |
| Chunk (3) | offset:u64 BE, 1–4096 bytes | Strict next-offset write |
| ACK (4) | next-offset:u64 BE | Permit one more application chunk |
| Finish (5) | SHA-256:32 | Verify exact size and digest |
| Complete (6) | empty | Confirm verified publication |
| Cancel (7) | empty | Discard unfinished work |

The sender reads at most 4 KiB at a time from the selected `File`. It waits for
the receiver's write ACK before sending another chunk. `SUD1` handles fragment
loss, reordering, ACK loss and duplicate suppression; the file protocol enforces
consent, offsets and integrity. This deliberately favors small memory/queue use
over bulk throughput. The existing user-data budget remains **2% of video wire
budget, capped at 64 KiB/s**, below input/control, audio and video, and charged to
the shared wire credit. Transfers on slow links can take substantially longer.

Offer consent expires after 90 seconds. Active transfers expire after 30 seconds
without progress. A missing completion confirmation does not prove the recipient
failed to save the file; check the receiving panel before offering it again.

Rust owns a dedicated per-child worker for disk writes, incremental SHA-256,
timeouts and cleanup. UDP/native receive and the global engine mutex never run
disk/hashing work. Worker queues, private stdin records and file messages are
bounded. The stdout reader only parses/queues private records, never blocks on a
disk operation, and strips **all `SNUSER_` payloads before diagnostics/logging**.
Names, paths and bytes are not emitted as diagnostic samples.

Mobile bridges pump at most four inbound and four outbound application messages
per 80 ms tick, entirely outside Flutter. The C ABI receive queue is capped at
16 messages; overflow aborts file work rather than terminating media. Native
transport and Rust worker handles carry generation fences. Swift/Kotlin file
actions use a utility executor separate from input/session teardown. Both JNI
and Swift preserve UTF-8 names including emoji; Android uses byte arrays instead
of JNI modified UTF-8 strings. Only metadata crosses MethodChannel.

Native file IPC records carry the authenticated remote generation. The worker
also stamps queued actions and incoming records with that generation; stale
commands/replies cannot be applied after reconnect or to another engine/session.
Closing the child closes the file service before waiting for the process.
Single-host private commands belong to its `Peer` worker; group membership
commands remain owned by `HostOutput`, avoiding competing queue consumers.

After acceptance, the receiver creates an exclusive `.part` file with user-only
permissions on Unix. It writes bounded chunks, checks exact size and SHA-256,
syncs/closes the file, then publishes with `hard_link` to a unique destination.
This operation cannot replace an existing path. On publication failure the
owned partial is removed; an existing destination is untouched. Filesystems
without hard-link support report an error rather than falling back to overwrites.

## Validation and next work

Rust tests cover consent before file creation, bounded in-flight chunks, ACK
progress, empty/reverse transfers, integrity failure, cancellation, timeouts,
existing destinations, stale generations and cleanup on service close. Tests use
random owned directories under the test temp directory, not the user's Downloads.
Native authenticated loopback tests inject packet loss/duplicates, check both
directions, forged capability/view-only denial and continued video/key progress.
UI tests cover invitation/accept/decline, metadata-only offers, pending/stale
operations and cancelling an upload when its panel closes.

The full native/Rust/desktop/Flutter suites run on the development Mac. A mobile
Rust C-ABI test transfers a selected cache source through two real shared workers,
checks content and Unicode names, then rejects a dead handle. The mobile C++
contract test checks desktop packets in both directions, stale generations and
view-only denial. Flutter tests check explicit offer/accept, pending/stale replies,
manual export and document UI lease cleanup. iOS is compiled as an unsigned device
build. All Kotlin sources are compiled with the project's Kotlin version against
real Android/Flutter/dependency JARs; JNI is syntax-checked with Android's official
header. This is not a complete Android native APK build or a device test.

Windows OS-specific builds, document providers and real phone/computer behavior
still need validation; no live throughput or latency improvement is claimed.

Next: full remote IME composition, then the remaining platform/voice roadmap.
These are separate from the
completed Unicode text editor and text clipboard. See [text clipboard](text-clipboard.md)
and [the UI roadmap](research/github-ui-platform-survey-2026-11-03.md).
