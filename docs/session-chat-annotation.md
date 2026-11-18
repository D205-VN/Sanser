# In-session chat and annotation

Text chat and temporary pen annotations during a session. Both work in 1:1
control sessions and in view-only group screen sharing (host + up to 3 viewers).

## Transport and why

Messages ride the native engine's existing **authenticated SNV2 user-data lane**
(`SUD1` chunked, acknowledged messages — the same lane used by the clipboard and
file transfer). This lane already exists on every 1:1 session and on each
per-viewer peer of a sharing host, so it reaches every participant without the
server seeing any content. The group-sharing signaling path (server polling)
was not used: it has seconds of latency, would put chat content on the server,
and needs new server endpoints.

- Capability: optional `SCO1` keepalive in both directions. Older engines ignore
  it, so the UI hides chat behind an "update Sanser" note.
- Message body: `SCL1` + a bounded JSON payload (≤ 4 KB native bound,
  `native/desktop/collab.h`). Native guard: 30 messages/s per peer.
- Shell IPC (stdin/stdout, kept out of logs and diagnostics by the existing
  `SNUSER_` private-output filter):
  - `collab-send\t<target>\t<hex>` — target `-` (single peer), `*` (all viewers)
    or a sharing viewer id.
  - `SNUSER_COLLAB\t<source>\t<hex>`, `SNUSER_COLLAB_CAP\t<source>\t0|1`.
  - `SNUSER_COLLAB_VIEW\t…` — viewer window geometry for the overlay.
  - `collab-exclude\t0|1` — macOS host overlay capture exclusion.

The **host shell is the hub** (`apps/desktop/src-tauri/src/collab.rs`): it
validates each viewer message, rate-limits it per sender, stamps the sender
(viewers cannot impersonate the host or each other), enforces the host's
annotation policy and relays to the other viewers.

## Limits

| Item | Bound |
| --- | --- |
| Chat text | 1 KB UTF-8, plain text, no control characters except newline/tab |
| Chat/clear rate | burst 5, then 1 per second per sender |
| Stroke points | 32 per message, 120 points per second per sender |
| Coordinates | fixed-point 0–10000 relative to the shared video (normalized 0–1) |
| History | memory only, 100 chat / 400 stroke events, cleared when the session ends |

Chat is rendered only through Svelte text interpolation, never as HTML.
Desktop notifications use the existing notification permission and never
prompt; they fire only while Sanser is in the background.

## Annotation overlay

A transparent, always-on-top, click-through Tauri window (`annotation.html`,
title "Sanser Annotations") draws strokes, which hold for 2.5 s and fade over
1.5 s. "Draw on screen" makes it accept the pointer until Done or Esc. It may
only call three overlay commands. Strokes are never drawn into the captured
video:

- Windows: `content_protected(true)` → `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`.
- macOS: `NSWindowSharingNone`, plus the host engine re-builds its
  ScreenCaptureKit filter excluding the overlay window (matched by the parent
  process id and window title).
- Viewers: the overlay is positioned over the aspect-fit video of the native
  viewer window using its reported geometry (macOS points / Windows pixels).

The host can turn annotations off for the group; the host then drops viewer
strokes and viewers leave pen mode.

## Needs checking on real machines

Automated coverage: native loopback test `sanser-collab-test`, Rust hub tests,
and Vitest for the helpers, store and panel. These were **not** verified on
real hardware:

1. **Windows host:** the overlay is absent from the shared video
   (`WDA_EXCLUDEFROMCAPTURE` needs Windows 10 2004+; older builds show a black
   box or the strokes). Desktop Duplication must honour the affinity.
2. **Windows viewer:** overlay alignment on a monitor with 125%/150% scaling and
   across two monitors with different DPI (the native window reports physical
   pixels).
3. **macOS host:** the log shows `SNV2_COLLAB_OVERLAY excluded=1` and strokes are
   not in the viewer's video, on macOS 13, 14 and 15. Transparent windows need
   `macOSPrivateApi` (enabled in `tauri.conf.json`).
4. **Host display:** the overlay covers the primary monitor. Confirm it matches
   the captured display (macOS main display; Windows Desktop Duplication output 0).
5. **Pen mode:** the overlay takes focus, Esc works, and clicks pass through again
   after Done. Check full-screen apps and Spaces on macOS.
6. **Viewer windows:** the overlay follows move, resize, minimize and close of the
   native window, and hides when the window is covered.
7. **Mixed versions:** an old viewer in a group shows the "update" note on its
   side and does not break chat for others; an old host shows the note on viewers.
8. **Relay path:** chat and strokes work over the UDP relay as well as direct.
