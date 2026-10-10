# Same-socket UDP echo (Mac client → Windows host)

Sanser 2.1.20 adds an independent, authenticated echo lane to the native SNU1/SNU2
single-socket media path used by the Mac client and Windows host. It is a diagnostic,
not a route-selection or congestion-control input. Update both computers.

The Mac receive pump sends one probe per second on the **same bound descriptor and
negotiated peer endpoint** used for video/input. It records a local monotonic time
immediately before `sendto`, after packet authentication has been computed. Windows
recognizes the fixed packet in its socket receive loop, authenticates it and sends
a same-size reply immediately without the JSON/control queue. The Mac records the
reply arrival immediately after `recvfrom`, before authentication, and accepts the
sample only after validation. Video consumption can be stopped without preventing
echo replies from being processed. Reporting uses a bounded latest-sample slot so
stdout/file logging does not run on the receive thread.

Overlay (Ctrl + Option + 8) adds **UDP echo RTT** and **Echo host hold**. Samples older
than three seconds disappear from the overlay. Connection details shows the latest
echo measurement and its timestamp alongside control Wire RTT. Numeric `SNU1_ECHO`
records are included in bounded local connection reports. Missing replies, old
hosts, disabled session authentication or unsupported native paths show no sample,
never zero RTT.

## What the measurements mean

- Echo RTT: local time immediately before send through socket reply arrival. It
  includes syscall/kernel/NIC/network delays, receiver scheduling and host work.
- Host hold: host receive through request validation and acquiring the shared send
  lock, ending just before response encoding/signing. Final response signing and
  the send syscall are still included in RTT; they are not included in host hold.
- Residual (report only): RTT minus that host hold. It is **not** kernel-timestamped
  wire latency, and it must not be compared as if it were one-way delay.
- Send call: local echo send syscall duration, retained in the numeric report.

Repeated low echo RTT with high control RTT is evidence to inspect the control path
and its instrumentation. If both are high, inspect the selected route and OS/network
queues too. Neither result uniquely identifies NAT, ISP QoS, Wi-Fi or a faulty NIC.
An 80-byte probe does not reproduce the size/load behavior of video datagrams. If
media is routed through a relay, echo follows that actual route when the bridge
supports the lane; it does not create a separate direct connection around a relay.

## Packet and trust boundary

The optional multiplex type `0x03` has an 80-byte fixed format: magic `SNE1`,
request/response kind, 64-bit probe ID, 64-bit random per-client-run nonce, host hold,
zero reserved bytes through the 64-byte header, then a 16-byte HMAC-SHA256 tag.
Direction-specific keys are derived from the existing session credential with
`sanser-udp-echo-v1/request` and `/response` contexts. Credentials, raw timestamps,
nonce and input contents are never logged. No unauthenticated echo mode is offered.

The host's connected media socket pins the peer. It permits at most four validation
attempts per second and pins the authenticated run nonce and increasing sequence.
The client pins the negotiated source, correlates only its pending IDs/nonce,
retains at most four probes and rejects duplicates, wrong direction/session,
modified/reserved bytes, impossible durations and replies older than three seconds.
Responses cannot amplify the request size. Echo does not consume or reset the
control/input or encrypted media sequence space. Older implementations ignore the
unknown lane. This is currently implemented for Mac client → Windows native host;
other native directions do not yet implement an echo responder/client.

## Tests and next steps

Shared protocol tests run on macOS and Windows. The Mac production receive-pump test
uses actual UDP sockets, checks the source media port, fills the video queue without
consuming it, rejects a bad HMAC, and verifies that the valid echo still completes.
Report tests retain only allowlisted numeric metrics and do not treat echo traffic
as proof that video started.

Reconnect both updated computers, exercise the desktop/game for 10–20 seconds, then
compare UDP echo RTT, Echo host hold, Control app RTT, Socket RTT, Wire RTT estimate
and Input ACK RTT. Export diagnostics to retain the measurements. Live standby
candidate probing, automatic reconnect/migration and kernel/NIC timestamps remain
separate work; this measurement alone does not lower a genuinely slow media path.
