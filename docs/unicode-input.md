# Unicode text and local IME composition

UI roadmap phase **3a**, implemented on 2026-11-03. This is source work following
the session action panel; it is not a new public release.

## Available behavior

On Android/iOS, open the remote session's **Bàn phím** panel, enter text locally,
then press **Gửi**. A supporting SNV2 Windows/macOS host receives Vietnamese,
combining marks, emoji and other valid Unicode text. The local keyboard/IME
finishes editing in this panel; its intermediate composition is not sent.

The panel shows whether Unicode has been confirmed by the host. Old hosts keep
the existing ASCII/HID path; unsupported text is rejected in full before a key
is sent. Sending is disabled while the command is pending, and the native bridge
checks the original session ID and rendered state. Reconnecting creates a fresh
capability state. Read-only sessions have no text editor or host input worker.

Phase 3b adds desktop committed text and permission-gated text clipboard:
see [Text clipboard](text-clipboard.md). Desktop/mobile [file transfer](file-transfer.md)
now shares a consented Rust worker. Neither feature synchronizes the host's IME
candidate UI.

## Local IME editing (phase 3e, 2026-11-04)

The mobile keyboard sheet now observes the whole `TextEditingValue`, including
composition-only and selection-only changes. Vietnamese accent composition and
other input methods run locally using the phone's keyboard. Intermediate
candidates and local selection changes send no remote input. While composition
is active, Send and remote key/shortcut actions are disabled; **Chốt chữ** or the
keyboard's Done action unfocuses the editor through Flutter, allowing its normal
finalization. This never sends text automatically. Review the resulting draft,
then press **Gửi**. The editor does not mutate the composing range inside a
controller listener.

Local editing arrows/backspace/selection operate on the draft; the separately
labelled remote-key buttons operate on the selected host field. No remote
backspace sequence is synthesized to repair autocorrection or candidate edits.
The native 256 UTF-16-unit limit is checked against the complete draft (a family
emoji or combining marks can use several units), without truncation or Unicode
normalization. The former grapheme-based `maxLength` could allow a draft the
native engine rejected. ASCII hosts retain full-draft validation before any keys.

Sending locks the sheet, releases Sanser-owned keys, drains earlier input and
checks the session again before queuing the committed edit. A rejected edit keeps
the draft and selection; no automatic application-level retry is made. Closing
the sheet or changing sessions fences late completion and never closes a newer
route. The sheet owns and disposes its controller with its route, replacing the
former delayed disposal timer.

Desktop Text and clipboard now respects browser composition start/end. Send and
clipboard actions cannot submit an intermediate candidate or replace the draft
during composition. Capability and session checks remain in force.

This is **local IME composition followed by Unicode commit**, not a TSF/
InputMethodKit bridge into the host application. Host selection, surrounding
text, marked text and the host's candidate window are not synchronized. Native
remote IME reconversion/selection requires a separately negotiated context-aware
protocol and OS integration; arbitrary remote backspaces are not an equivalent.

References: [Flutter TextEditingController](https://api.flutter.dev/flutter/widgets/TextEditingController-class.html)
(controller listeners, composing ranges and Gboard restoration),
[web compositionend](https://developer.mozilla.org/en-US/docs/Web/API/Element/compositionend_event).

## Native Mac viewer text mode (phase 3f, 2026-11-04)

The **SNV2 native Mac viewer** now supports the controller's AppKit input method
directly in its video window. Click the stream to acquire input, then use the
native title-bar **Text input** button or **Control+Option+I**. The action becomes
available only for an authenticated Unicode-capable, input-enabled host and an
active focused viewer. It is absent for view-only windows. Toggle back with
**Physical keys** or the same shortcut for HID/game input; a new window always
starts in physical mode. Control+Option+Escape still releases remote input.

`SanserSnv2RemoteView` is now an `NSTextInputClient` in `ime_view.mm`. It presents
bounded marked text and local candidate selection with an AppKit candidate-window
anchor near the last click. The anchor is approximate; it does not discover the
host application's caret. Candidate updates remain local. `insertText` or a normal
`unmarkText` commits one strict UTF-8 edit through `Peer::text` on the existing
reliable ordered input lane, with no extra Send button. Consumed text key-ups are
not also forwarded as HID. Ctrl/Command shortcuts, function/navigation keys,
Shift+arrow and Shift+click keep balanced HID modifier transitions in text mode.
While a candidate is active, its input method owns navigation instead.

Focus/release, input permission loss, clicking another remote location and
connection-generation changes discard marked text. Cancellation clears the
draft before asking AppKit to discard conversion, with a reentrancy guard against
callbacks committing it. `Peer::text` accepts an optional expected authenticated
generation and checks it under the same mutex as generation publication before
reserving any input sequence. A rejected/stale edit never falls back to physical
typing. Rejected input blocks further commits until explicit Esc/click/toggle
discard; it does not automatically replay when capacity returns. Only a bounded
valid local preview is retained, and its content does not enter telemetry.

The local input client has no access to committed text/selection in the remote
application. It therefore refuses replacement ranges outside the current local
marked region, rather than guessing remote backspaces or reconversion edits.
Context queries are bounded and expand to complete graphemes. This continues
to be **local composition + remote Unicode commit**, not host-side TSF/
InputMethodKit context synchronization. This direct Mac mode does not add IME
to the Legacy Mac receiver. The Windows viewer uses the separate draft composer
described below; it does not automatically stream each local commit as Mac does.

Source-only validation: the production native library and the combined
Legacy/SNV2 Mac executable build; AppKit tests exercise marked-text replacement,
selection/context queries, exact commits, cancellation, held modifiers,
generation/permission/focus fences, size/Unicode rejection and the native button
gate without posting OS input. The Unicode transport test also rejects stale
generation commits without leaving a reliable-input sequence gap. Automated
callback tests do not substitute for trying macOS Vietnamese Telex, Japanese/
Chinese candidates and target applications on two real machines.

API contract: [Apple NSTextInputClient](https://developer.apple.com/documentation/appkit/nstextinputclient).

## Native Windows draft composer (phase 3g, 2026-11-05)

The SNV2 Windows viewer now has a modeless **Text input** action in its window's
system menu (Alt+Space), also opened with **Ctrl+Alt+I**. It requires an
authenticated Unicode-capable control host; view-only windows install no action.
AltGr+I is excluded from the toggle. The ordinary physical HID path remains the
default for games and remote keyboard layouts.

The panel contains a native multiline Win32 `EDIT`. Windows owns local keyboard/
IME composition, candidate UI, selection, undo, paste and backspace. Nothing in
that local draft changes the host until **Send** or **Ctrl+Enter**. Enter alone
adds a local line break. While an IME candidate is active, Send is unavailable;
Enter/Escape remain with the IME. After composition ends, Escape/Cancel closes
the draft. Ctrl+Alt+Escape cancels the draft and leaves remote input released.
Closing/canceling the panel does not disconnect video/audio. Click the stream
to resume physical remote input. A click outside the panel discards the draft
before changing the remote target, avoiding text being submitted into a new field.

Opening releases Sanser-owned remote input. Edit and button messages remain in
the local panel; dialog Tab navigation is integrated with the bounded viewer
message pump. The view keeps painting frames behind the panel. A switch between
child editor/buttons preserves the draft; focus leaving the panel fences it
immediately and posts a cancellation. Those internal posted messages carry a
panel serial, so a late cancel cannot close a newer draft. Button submission
checks its current control handle. Capture/input pause, lost permission and
authenticated peer-generation change also discard the draft. Submission checks
the active window/focus, permission and original generation again before calling
the generation-fenced `Peer::text`. It never automatically retries a rejected or
ambiguous edit, and rejection preserves the draft and selection for review.

The editor permits a larger local draft, but final submission rejects a complete
edit exceeding 256 UTF-16 units or containing malformed/control text. Ordinary
typing is limited to 4,096 local units; oversized programmatic/provider content
is refused without reading an unbounded buffer. `imeCommitUtf8` converts only
valid scalars and Windows CRLF to protocol LF, preserving combining marks and
emoji. It never truncates a draft or guesses backspaces in the remote document.

Validation for this source increment:

- The native panel and Win32 test executable compile/link both through CMake
  and a standalone Windows compiler with warnings treated as errors, including
  without the `UNICODE` macro. The integrated Windows backend passes syntax
  checking. These are cross-build checks on Mac, **not executed Windows tests**.
- The portable converter runs on Mac, including ASan/UBSan, covering exact
  Vietnamese/combining/emoji text, CRLF, empty/control/malformed text and the
  256-unit boundary. Unicode transport and AppKit regression tests also run.
- Windows test cases cover explicit submission, candidate gating, AltGr,
  Ctrl+Enter, draft/selection retention, stale focus/generation, provider length
  bypass and delayed cancellation. They are registered for Windows CTest/CI;
  their runtime result is still pending on Windows.
- A complete Windows client cross-build is not verified: the available MinGW
  SDK lacks existing audio-capture WRL headers. No production audio or SDK code
  was changed to work around this. Real Windows SDK/MSVC build and physical
  Microsoft Telex/Pinyin/Japanese IMEs, third-party input methods and target
  applications need validation before release.

The panel is a separate `sanser-windows-text-input` library, allowing UI tests to
build independently of capture/codec SDK dependencies. Configure
`native/platforms/windows/text-input` as a standalone CMake project to build/run
the two tests without WRL, capture or audio dependencies. The `windows-ime` CI
job now runs this independent project on Windows; its remote result remains
pending until the source is pushed. There is no native wire
format change or new bundled runtime dependency. This remains local composition
plus explicit Unicode commit; it does not synchronize host IME/context or add
mobile controller functionality. [Mobile physical keyboard](mobile-external-keyboard.md)
is a separate phase-4 increment and uses the existing HID path.

Primary API references: [Microsoft edit controls](https://learn.microsoft.com/en-us/windows/win32/controls/edit-controls),
[edit text operations and limits](https://learn.microsoft.com/en-us/windows/win32/controls/edit-controls-text-operations),
[IME composition and messages](https://learn.microsoft.com/en-us/windows/win32/intl/ime-messages).

## Transport contract

- Host announces the authenticated `SUT1` Keepalive capability only for an input
  enabled, non-sharing session with an input callback.
- `Keyboard` packet on reliable ordered **stream 2**: byte `5` (`Input::Text`),
  followed by strict UTF-8. Other input kinds retain their existing wire format.
- An edit is limited to 1,024 UTF-8 bytes and 256 UTF-16 units, fitting one UDP
  datagram with the 80-byte SNV2 header. No bulk data/fragmentation uses this lane.
- Reject empty/oversized input, invalid UTF-8, overlong encodings, lone surrogates,
  out-of-range scalars, NUL and control characters except LF and Tab. LF/Tab
  become ordinary Enter/Tab input on the host.
- Reuse input ACK/SACK, ordered recovery, duplicate suppression and input reset
  fences. A text edit cannot reserve a sequence before capability/validation
  succeeds. A full pending window rejects the edit without stopping video.
- The C ABI returns **queued**, not an acknowledgement that an application typed
  the text. Text contents never enter telemetry. Actual OS injection remains on
  the input worker rather than the UDP receive or video decode thread.

## Platform injection

macOS resets Sanser-owned held keys/buttons, then emits key-down/up events per
Unicode scalar with `CGEventKeyboardSetUnicodeString`. Surrogate pairs stay in
the same event. Windows uses `SendInput` with `KEYEVENTF_UNICODE`/UTF-16 units,
retaining failed key-up events for reset. It aborts the edit if held-key releases
fail, rather than typing through a stuck Ctrl/Alt modifier. An OS failure can
still occur after a prefix is typed; the transport must not automatically retry
an already delivered edit at application level.

The existing authenticated Windows helper accepts only the fixed Unicode
keyboard event representation (16-bit unit and Unicode/optional KeyUp flags),
with the same caller checks and desktop policy as other input. Updated host and
helper binaries are needed for elevated windows. No UAC policy is disabled.

Applications can ignore text events (for example, input driven solely by physical
keycodes); use the existing HID shortcuts/game controls there. Reference APIs:
[Microsoft KEYBDINPUT](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-keybdinput)
and [Apple Unicode keyboard events](https://developer.apple.com/documentation/coregraphics/cgevent/keyboardsetunicodestring(stringlength:unicodestring:)).

## Validation

Portable native tests cover malformed/oversized text, Vietnamese/combining marks/
surrogate pairs, authenticated negotiation, dropped-first-datagram repair,
ordering behind a missing text edit, duplicate suppression, an old host that
does not announce Unicode, and forged text sent to a read-only/sharing host.
The mobile C ABI test passes Unicode to a real loopback desktop peer. The macOS
injector test inspects fake events without posting real OS input. Windows native
tests include Unicode event fields and malformed-payload rejection; the portable
helper protocol validator is runnable on macOS too.

Flutter tests cover exact text/session payloads, invalid edits with no partial
send, no ASCII fallback after native rejection, and the pending-send button.
Phase 3e adds candidate replacement without network traffic, blocked remote
shortcuts during composition, explicit finalization without sending, selection
preservation after native rejection, emoji/combining-unit boundaries, disconnect
during input drain and closing while a reply is pending. Desktop tests cover
IME finalization/cancellation, clipboard gating and session reset. Automated IME
updates are simulated framework/browser events, not physical keyboard coverage.
Physical iOS/Android typing and Windows injection still need device validation.
An unsigned iOS compilation verifies the Swift/C ABI bridge; Android full build
and Windows OS-specific tests need their respective SDK environments.

## Remaining phase 3

1. [Desktop file transfer](file-transfer.md) now implements offer/accept/cancel,
   bounded chunks/in-flight window, integrity and safe destinations on a separate
   low-priority path. Mobile document pick/export now uses the same Rust worker;
   provider/device validation remains before release.
2. Native local input is implemented in source for both SNV2 desktop viewers:
   Mac direct IME commits, Windows draft/explicit Send. Windows runtime and
   physical keyboard/target-app validation remain. Full host-context IME
   composition/selection is separate: no viewer mirrors the host's surrounding
   text, marked text or candidate UI. Legacy remains on its physical input path.
