# Cross-direction UDP stability

Scope: improve resilience of the existing engines, without replacing codecs,
changing route negotiation or increasing video buffering. This source change
does not update an installed 2.2.24 application or publish a release.

| Controller → host | Path covered by this change |
| --- | --- |
| Mac → Windows | Legacy Windows video sender and Mac UDP receive pump; SNV2 when selected |
| Windows → Windows | Shared SNV2 sender/receiver |
| Windows → Mac | Shared SNV2 sender/receiver |

## Findings

- SNV2 ignored the return value of `send()`, so socket failures were invisible
  to transport diagnostics. Receive errors of every kind were ignored, including
  invalid handles which can return immediately and cause a busy loop.
- The legacy Windows video sender threw on any failed datagram, including
  temporary buffer pressure or route errors. Its existing fragment repair cache
  could still contain the picture, but the exception interrupted transmission.
- The legacy Mac receive pump treated any socket read failure other than
  interruption/would-block as terminal. `POLLERR` was also terminal without
  consuming/classifying the socket's pending error.

These are verified control-flow gaps, not proof that they caused every reported
freeze, audio problem or RTT spike. This change does not explain a particular
physical network's 200 ms latency or fix game-specific input rejection.

## Changes

1. Share one UDP-specific error policy across both engines. Interrupted calls,
   temporary buffer pressure and route/ICMP errors allow protocol recovery.
   Receive timeout/would-block remains ordinary idle work, not an error count.
   Invalid handles/arguments and oversized outbound datagrams remain terminal.
   A truncated inbound datagram is discarded; it is never decoded as valid media.
2. SNV2 counts failed sends and temporary receive errors. Reliable input retains
   its sequence and existing ACK/retry deadline. Missing video uses existing
   NACK/cache repair. Audio retains its freshness limits. Terminal send/receive
   failures now become explicit transport errors and clean session shutdown.
   No error renews authenticated peer liveness or releases the input safety rules.
3. The legacy Windows video sender stops at the first failed fragment and returns
   the successful prefix. The cached frame remains available for bounded NACK
   repair. Repair resumes at the failed index, preserving authentication counters
   and avoiding a silently skipped fragment or an unbounded socket retry loop.
   Legacy audio now uses the same transient-send classification.
4. The legacy Mac receive pump consumes `SO_ERROR` on `POLLERR`, continues after
   temporary datagram errors, and keeps invalid sockets terminal. Temporary
   receive failures sleep 1 ms to avoid spinning; this adds no delay to healthy
   reads. Existing source filtering, queue capacity and stale-video expiry remain.
5. Numeric `sendTransientErrors`/`receiveTransientErrors` counters are retained in
   bounded connection reports. Socket identifiers, endpoints and tokens are not.

## Validation

- Production Mac host/client and all native targets compile in an isolated
  temporary build, leaving the workspace's staged sidecar untouched.
- Authenticated localhost fault injection covers Auto and Ultra: temporary
  outgoing video loss, failed reliable key-down/up sends, receive route errors,
  NACK repair and continuation of the same P-frame chain. It also checks clean
  shutdown on an invalid receive socket, invalid send socket and partial send.
- The production legacy Mac receive pump survives two injected read failures
  and delivers the queued datagram on the same socket. Its invalid-socket test
  continues to report the failure to the consumer.
- Windows legacy media tests additionally inject four transient error types
  midway through a three-fragment picture, verify the successful prefix, resume
  repair at the missing fragment, and verify terminal invalid-socket handling.
  Windows execution is verified separately by CI, not by a local Mac run.
- Connection-report tests verify only numeric error counts are retained.

Local result: **33/33 native tests** and **27/27 connection-report tests** passed;
Rust formatting and whitespace checks passed. Windows results remain pending
until the branch CI build completes.

Physical three-direction soak testing is still required: Direct LAN/WAN and
available relay paths, 15–30 minutes per direction, with moving content, input,
audio and output-device changes. Measure freeze duration/recovery, disconnects,
RTT P50/P95, dropped frames and audio underflows. Localhost results cannot prove
that all three directions are stable on school Wi-Fi, NATs or real GPUs.

## References

Microsoft documents UDP-specific ICMP errors separately from normal receive
timeouts and invalid sockets in [recv](https://learn.microsoft.com/en-us/windows/win32/api/winsock/nf-winsock-recv)
and buffer/resource/argument errors in [send](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-send).
The recovery policy applies to datagram sockets only; it is not applied to WSS/TCP.
