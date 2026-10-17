# Part 4: bounded input delivery and independent OS execution

Included in Sanser 2.2.13, developed on `Version-2.0.0` after 2.2.12.
This follows the shared network foundations, performance diagnostics and bounded
route reconnect work. It does not replace OS capture/codec/render backends.

## Implemented behavior

SNV2 now has one dedicated host input worker. The UDP receive thread authenticates,
orders and queues input, then sends a receipt ACK without waiting for an OS input
call. A slow injection therefore does not hold UDP control/pong, receipt ACKs or
video receive processing. Only an input-enabled host starts the worker; view-only
hosts still acknowledge denied input without executing it.

The worker keeps at most 256 waiting actions. Adjacent absolute mouse moves keep
the newest position, but coalescing never crosses a button, keyboard, scroll or
reset boundary. Reliable actions stay ordered. Capacity pressure can evict a
disposable move; an action overflow instead stops the peer and releases held
state. Focus/reset discards stale queued actions. Shutdown discards queued presses
and performs a final reset on the same thread that owns OS injection. Exceptions
are contained, stop the peer, and attempt a final reset.

The sender still reserves independent control, reliable-input, latest-move,
audio, video and repair queues. Each reliable packet now has at most one queued
send/retry. Its retry clock and attempt counter begin when the sender dequeues
it for the socket, rather than when it was initially enqueued. Cumulative ACK
and selective ACK also discard obsolete queued sends. Input and ACKs bypass
video pacing. Opportunistic video-cache repair no longer waits for a publisher
holding the video mutex in the UDP receive loop.

`native/common/input_retry.h` supplies the bounded retry/backoff formula to both
Legacy and SNV2. Legacy uses fresh control RTT, as before; SNV2 smooths unambiguous
input ACK RTT and its variation. Retransmitted ACKs are excluded from the SNV2
retry estimator because the matching send attempt is unknown (Karn's rule).
Repeated losses back off rather than creating a fixed-rate resend storm. Retry
spacing is 8–500 ms before backoff, capped at one second after backoff; the SNV2
overall unacknowledged-input deadline remains 2–5 seconds. Legacy retains its
four-attempt reset behavior and existing authenticated JSON wire format.

## Measurement semantics

**SNV2 input ACK now means acceptance into an ordered execution queue, not proof
that the OS has injected the event.** A low ACK RTT alone cannot certify responsive
control. Diagnostics displays the execution stages separately:

| Measurement | Meaning |
| --- | --- |
| Input ACK RTT | Enqueue to authenticated receipt ACK, including retries and local queues |
| Input send queue | Time waiting for the sender to select an input datagram |
| Host input queue | Time from accepted queue insertion to start of OS injection |
| Host input work | Duration of the OS injection callback |
| Host input queue maximum | Largest queue wait in the reporting interval |

The authenticated optional `SIT1` feedback extension carries numeric host queue
and work durations back to the client. It contains no keys, pointer coordinates,
text, credentials, wall-clock timestamps or packet contents. Old clients ignore
this extension; the existing ACK/SACK payloads and input serialization are unchanged.
Reports keep retry/coalescing counts as counts, not milliseconds. RTT names remain
distinct from wire/network RTT, and absent or stale execution timings are unknown.
P50/P95 still summarize periodic observations, not per-event percentiles.

## Validation and remaining limits

Synthetic tests block an injection callback and verify that authenticated control
pong, reliable receipt ACK and video still pass on the same UDP socket. They also
verify queue bounds, 10,000-move coalescing, action order, key-up, reset at capacity,
shutdown release, callback failure, retry/backoff and ACK/SACK queue cleanup.
No test posts physical keyboard/mouse input or captures a real screen.

Both SNV2 host backends use the same worker; macOS still uses CGEvent and Windows
still uses SendInput. Legacy gets the shared retry rule but keeps its existing
input worker/JSON batching and injection path. This is not a complete Legacy/SNV2
wire migration. Gamepad, relative mouse, clipboard and mobile input are not added.
Group sharing remains view-only/video-only. A blocked OS injection itself cannot
be cancelled; reset waits for that in-flight callback to return. Capacity bounds
prevent unbounded memory, not a guarantee that every OS call finishes on time.

Physical Mac→Windows, Windows→Mac, Mac→Mac and Windows→Windows tests must still
exercise typing, modifiers, clicks/drag, focus loss, held keys during Disconnect,
view-only isolation and input responsiveness under video congestion/loss. Record
ACK RTT together with host queue/work and control RTT. These changes do not prove
that a 200 ms Internet path has become a 10–15 ms path.
