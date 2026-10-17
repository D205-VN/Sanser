# Part 2: common native performance diagnostics

Development on `Version-2.0.0`, after the 2.2.12 release. No new installer or tag
is required for this work. The installed 2.2.12 artifacts do not contain it.

The shared `sanser-diagnostics::NativePerformance` model normalizes allowlisted
numeric observations from both media implementations. OS capture, codec, render
and input backends remain native. This does not replace Legacy with SNV2, deploy
a relay, migrate routes, or claim to fix a physical 200 ms path.

## What is connected

1. Legacy control, input ACK, capture/encode, decode and presentation telemetry
   maps into common stage names. The misleading legacy `SNINPUT_RTT` record is
   recognized as a **control-pong** measurement, not input delivery latency.
2. SNV2 clients send one optional authenticated control probe per second through
   their existing media transport. A matching host responds on the control lane.
   The measurement includes TX queues, crypto, peer handling and RX scheduling;
   it is **application RTT**, not wire RTT or one-way latency. Probe state is
   bounded to four requests with a three-second expiry. Unknown IDs, duplicate
   responses, stale responses and responses from a previous peer generation
   cannot become current measurements. Old peers safely ignore the extension.
3. SNV2 reliable input ACK timing is separate from control timing. SACK counts
   an event only when first acknowledged; a later cumulative ACK does not count
   it again. The host records the work of allowed input callbacks locally.
4. The shared decode worker measures enqueue-to-decode-start separately from
   time spent in its decode callback. Both Windows and macOS SNV2 clients emit
   these periodic averages. Windows conversion/paint and Mac encode+publish
   remain separately named; combined timings are not relabeled as pure stages.
5. SNV2 numeric telemetry records are written as complete stdio records so peer,
   decoder and UI threads cannot splice their numeric fields together.
6. Connection reports contain a common `performance` summary and findings,
   alongside the original raw numeric samples/maxima for compatibility. The
   reports use schema 2 and preserve the existing 20-file/256-KiB-per-file caps.
   No packet contents, keystrokes, screen images or credentials are added.
7. A bounded live snapshot reaches Settings → Diagnostics → Measured native
   performance. It separates hosting from viewing, includes the confirmed
   Direct/Relay transport and excludes events from an older native process.
   Export diagnostics also includes these snapshots. Metrics older than five
   seconds, or belonging to an ended process, are not displayed as live values.

## Measurement meaning and availability

| Measurement | Legacy Mac viewing Windows | Shared SNV2 |
| --- | --- | --- |
| Control application RTT | Control timing or control-pong fallback | Authenticated probe on the current transport |
| Wire/socket RTT | Existing detailed control instrumentation | Not measured by this change |
| Input ACK RTT | Legacy input acknowledgement | Reliable ACK/SACK elapsed time, including retries |
| Host input work | Host timing carried by legacy ACK | Local host report only |
| Capture/encode | Existing host stage messages | Mac host encode queue and encode+publish locally; unsupported stages stay absent |
| Decode queue/work | Existing decoder work; queue unavailable if not emitted | Shared decode worker on Windows and Mac |
| Frame queue/GPU render | Existing Metal measurements | Unavailable if the backend does not emit them |
| Color conversion/paint | Unavailable if not emitted | Windows client measurements |

SNV2 is used by Windows→Mac, Mac→Mac, Windows→Windows and multi-computer
sharing in both directions. Ordinary Mac→Windows remains Legacy. A protocol
adapter maps available observations; it never synthesizes missing ones as zero.

P50/P95/max are computed from the last **120 observations per stage per process
launch**, using nearest-rank percentiles. Some observations are periodic or
lifetime averages from an engine. They are not per-frame percentiles, a bandwidth
benchmark, or screen-to-screen latency. Combined stages overlap; adding all rows
would double-count work. Zero is retained only when actually reported.

Findings identify high control RTT, input delivery overhead relative to a fresh
control sample, capture/decode work over the requested frame budget, and queue
pressure. Capture wait is not capture work. A high application RTT without wire
instrumentation is never blamed specifically on the network/router/ISP. Findings
are local diagnostic hints, not automatic route changes or congestion decisions.

## Validation and remaining work

Automated checks cover normalized semantics, bounded percentile windows,
rejected invalid/stale values, credential exclusion, report persistence and GUI
notifications without a GUI runtime, old-process isolation, and missing/stale UI
values. Native checks cover authenticated loopback probes while video/input are
active and decode worker timing snapshots. These use synthetic data, without
screen capture or real OS input injection.

Real two-computer tests in all four directions, Windows compilation, NAT/relay
behavior and physical screen-to-screen latency still need their respective
verification. Kernel timestamps, SNV2 wire RTT, remote host-stage propagation,
complete per-frame histograms and unified Legacy/SNV2 transport are later work.
