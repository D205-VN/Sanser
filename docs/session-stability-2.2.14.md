# Session stability follow-up (2.2.14)

This patch follows the reported cursor duplication, input ACK timeout, viewer
close, shared video freeze and Relay/installer failures. It separates measured
application bugs from causes that still require physical machines/server logs.

| Report | Implementation | Remaining validation |
| --- | --- | --- |
| Windows controlling Mac shows a trailing pointer | ScreenCaptureKit includes the cursor only for view-only/sharing captures | Fast pointer, click and drag on physical Mac/Windows |
| One unacknowledged input closes video | Optional authenticated input-only reset, sequence fence and host input lease | Continuous real input on Direct/WSS/UDP Relay |
| Viewer X leaves an accepted session | Native viewer-close marker, persisted owning session, server Disconnect and isolated cleanup | Installed Windows/macOS windows, including main-window tray behavior |
| Share loses video after dispatch pressure | Bounded encoded reference chain per viewer, existing overload/IDR backoff and isolation | 1–3 real viewers for ten minutes, including a constrained viewer |
| Relay reset / installer pause | Preserve terminal Relay cause, bounded reauthorization, graceful server restart, authenticated DXGI pause state | Actual UAC 15–30 seconds, deployed server restart and WAN proxy failures |
| Group sound missing | Deliberately still disabled | Dedicated group audio capture/codec/distribution work remains |

## Input reset contract

`SIC1` advertises the extension within authenticated SNV2 keepalives. A client
with input enabled sends `SIE1` lease renewals while input is active. At reliable
input expiry (the existing adaptive 2–5 second deadline), a supporting client
stops generating input, clears its input TX lane and sends `SIR1` containing a
reset serial and its highest issued reliable sequence. Host accepts a forward
fence within its existing 64-action window, clears pending actions, schedules an
OS input reset on its injection worker, and returns `SIA1` with the same serial
and fence. A duplicate reset is acknowledged without resetting later actions.
Old actions at/below the fence cannot inject after recovery. No key identity,
text or coordinates are added to numeric reports.

The matching authenticated confirmation is required within five seconds.
Otherwise input stays disabled and a native title asks for reconnect; video and
control may continue. The host releases held input after two seconds without a
renewal. Receipt/reset ACK is not a claim that an OS API completed successfully.
If an OS injection call itself blocks, it cannot be forcibly cancelled by this
protocol. Old peers that do not advertise the extension still fail closed at
input expiry. View-only permissions remain enforced at the host and viewer.

## Relay failure and restart

`relay connection failed: WebSocket protocol error: Connection reset without
closing handshake` says the WebSocket transport ended abruptly. It does not
identify whether a proxy, network, server process or another fault caused it.
The status poll must check for a recorded Relay failure even if input/peer
timeout has already terminated native. Preserve that cause with its session ID,
not with the next engine launched under the same role.

A dead accepted route may wait through an unavailable server for at most thirty
seconds. No candidate/media engine starts until fresh server readiness and
credentials succeed; 401/403 and non-transient failures stop automatic recovery.
The existing two-route-reconnect budget, Disconnect cancellation and ownership
checks remain. A working slow route is not closed while consent is unavailable.

During controlled server shutdown, WebSocket Relay/control receives Close 1012.
Close writes have finite deadlines. Policy close 1008 is not retried as a restart.
This improves diagnosis and orderly deployment; it cannot prevent an external
TCP reset. The server change must actually be deployed to take effect.

## Group frame queue and capture pause

Raw captures may use latest-frame replacement. Already-encoded P pictures cannot
be replaced independently of their references. Each sharing delivery now keeps
up to eight encoded pictures / 8 MiB, consumes them without a deliberate hold,
and supersedes queued references only with an IDR or whole-chain overload reset.
The existing shared encoder, separate peer crypto/repair and slow-viewer policy
remain. This fixes one dispatch failure mode, not every decoder/network freeze.

SNV2 Windows capture recovery now announces `SCP1` state independently of video.
The client suppresses input while capture is unavailable; the host acknowledges
but does not inject control into the unavailable desktop. Cached GPU frames are
discarded and a fresh IDR is requested after DXGI resumes. Legacy already has
capture-paused control telemetry. Neither path gains Secure Desktop privileges.

Automated tests exercise synthetic 25-second capture silence and WSS media
silence while authenticated control/heartbeat remain active, plus resume. These
do **not** simulate a real GPU reset, capture permission dialog or UAC desktop.

Group audio should follow only after this video acceptance testing: capture once,
encode Opus once, independently authenticate/fan out to opted-in viewers, with a
bounded queue and mute for each. Removing `audio=false` alone is not sufficient;
no group-audio capability is advertised by this patch.
