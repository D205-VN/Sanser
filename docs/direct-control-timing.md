# Direct UDP control/input timing

This instrumented Windows host → Mac client path keeps the existing multiplexed
UDP socket, scheduling, Auto/Ultra policies and media protocol. Select **Direct**
for the baseline and verify the negotiated route in the connection report.
The extension can also measure the local UDP bridge used by relay; those samples
must not be called Direct or compared without their route metadata.

## What is measured

| Marker | Clock | Location |
| --- | --- | --- |
| T0 | Mac monotonic | Entry to sender enqueue, before sender mutex |
| T1 | Mac monotonic | After authentication/serialization, immediately before `sendto` |
| T2 | Windows monotonic | Immediately after UDP `recv` returns |
| T3 | Windows monotonic | Immediately after control queue pop, before authentication |
| T4 | Windows monotonic | After reply authentication/serialization and socket mutex acquisition, immediately before `send` |
| T5 | Mac monotonic | Immediately after UDP `recvfrom` returns |
| T6 | Mac monotonic | Reply processing after control demux queue and authentication |

The host sends T2/T3/T4 in a **separate control-timing receipt** after the original
pong/ACK has been sent. It uses the existing authenticated control envelope.
The receipt's arrival time is **not** used as T5/T6. Losing/reordering a receipt
may lose a measurement, never extend a ping or block input waiting for telemetry.
Legacy peers continue to work but have no decomposed timing (overlay shows `—`).

- `appRttMs = T6−T0`
- `macSendQueueMs = T1−T0` (includes preparing/authenticating the packet)
- `hostReceiveQueueMs = T3−T2`
- `hostControlWorkMs = T4−T3` (includes authentication, input handling, reply
  construction and waiting for the shared send mutex)
- `macReceiveQueueMs = T6−T5` (includes demux and authentication)
- `hostSendLockMs`: time waiting for the Windows shared UDP send mutex (a subset
  of host control work). `hostSendCallMs`: duration of the pong/ACK `send` syscall
  after T4. These are diagnostic subsets/overlaps; do not add them to app RTT.
- `socketRttMs = T5−T1` (includes host application residence)
- `wireEstimateMs = (T5−T1)−(T4−T2)`

**Wire RTT estimate includes OS/socket queues and receiver scheduling before
`recv` returns.** In particular, Mac video/decode work before the next `recvfrom`
can increase this residual. It does not prove that the Internet is slow. These
are userspace markers, not NIC/kernel packet timestamps. No one-way delay is
computed from the unsynchronized Mac and Windows clocks.

Control ping application RTT remains the conservative input for retry/congestion
policy. Input ACK samples no longer enter the control RTT accumulator. This
change intentionally does not retune adaptation using the new residual.

Input batches are sampled at most four times per second; every ping is sampled.
A batch's T0 is the oldest retained event enqueue timestamp. On retries it still
refers to original enqueue, so `macSendQueueMs` includes retry waiting; T1 is fresh
for each sampled attempt. Raw unacknowledged pointer moves are not ACK probes.
Coalesced events whose original timestamp was discarded use batch creation as
fallback. `Input ACK RTT` retains its existing batch-creation-to-ACK meaning;
`SNINPUT_TIMING.appRttMs` also accounts for the pre-batch queue when available.
Each send attempt gets a unique ID; duplicate/unknown replies, impossible
intervals, stale samples and missing receipts never produce invented timings.
Pending storage is capped at 128 records. Receipts over three seconds late do
not refresh the overlay.

## Testing on the affected machines

1. Build/update **both** Windows host and Mac native client from this change;
   updating only the UI or one native executable is insufficient.
2. Connect with Direct selected. Keep resolution/FPS/bitrate/Ultra settings
   unchanged during the baseline. Confirm Direct in the report; don't infer it
   just from private/local bridge socket addresses.
3. Toggle the overlay with **Ctrl+Option+F8**. Record idle desktop and active
   window movement separately for at least 60 seconds each. Include continuous
   pointer movement plus key/button presses for ACK samples.
4. At the same time run `ping <Windows-LAN-IP>` on Mac for a LAN baseline.
   Use the actual reachable peer endpoint for WAN tests. ICMP and UDP need not
   follow identical queuing rules.
5. Inspect `SNCONTROL_TIMING` and `SNINPUT_TIMING` in native logs. Raw logs include
   T0–T6. The existing local connection report retains only allowlisted numeric
   durations, not raw timestamps, session secrets or input content. Reports keep
   a bounded rolling history (120 total samples across all metric sources), so
   capture logs during a longer run rather than interpreting the report as a
   complete session trace.

Use the decomposition to choose the next change: Mac sender queue, Windows
control queue, host processing/send lock, Mac demux queue, or the residual.
A large residual requires distinguishing network/kernel backlog from work before
socket receive. Don't split sockets, change Auto repair deadlines or tune relay
based solely on the application RTT.

Acceptance targets for the user's networks (must be measured, not inferred from
a loopback test): ICMP roughly 6–12 ms, socket/residual RTT roughly 10–25 ms,
input ACK 10–30 ms, host input under 5 ms and frame queue under 3–5 ms. Keep P50/P95,
route and sample count alongside any averages. No physical two-machine result is
claimed by the automated tests.
