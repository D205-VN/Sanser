# Direct reliability after 2.1.15

This change is confined to candidate gathering. It does not replace relay,
change input scheduling, or tune Auto/Ultra buffering.

## Fixed failure cases

- A lost initial STUN request previously exhausted that server's transaction
  without a retry. The same transaction now retries after 500 ms with exponential
  backoff, within the caller's existing deadline. Replies must match the queried
  IP **and port**, and the transaction ID. See
  [RFC 8489, section 6.2.1](https://datatracker.ietf.org/doc/html/rfc8489#section-6.2.1).
- A host with many interfaces could fill the signaling cap with host candidates
  and retain only one public endpoint. Selection now preserves a candidate from
  each address-family/origin class before filling the remaining priority slots.
  STUN, UPnP and manual candidates remain distinct when their endpoints differ.
- Desktop signaling excludes IPv6 candidates while the native media launch path
  only supports IPv4. An unusable IPv6 candidate must not evict an IPv4 route.
  This does not add native IPv6 media support.

## Validation and limits

Loopback tests drop the first STUN exchange, inject a reply from the same IP but a
different port, inject an old transaction response, and verify socket/transaction
reuse. Selection tests cover many host interfaces, deterministic ordering,
distinct public routes, and duplicate endpoints. Existing authenticated pairing,
late-peer and peer-reflexive endpoint tests remain in the suite.

These tests do not establish traversal success through physical NATs, Windows
firewall permission for a native sidecar, or media startup after the probe socket
is handed to the native process. Those are separate acceptance checks. Record
the actual route and first decoded frame on two updated machines in LAN, then
across networks. A failed probe alone cannot identify the firewall as its cause.

## Next transport work

The UDP/QUIC relay is still unimplemented. Its implementation needs authenticated,
expiring session allocations, end-to-end media encryption, bounded datagram queues,
input/control priority and loss/congestion tests before becoming the default.
Deployment requires a reachable public UDP endpoint; the present WSS relay remains
the deployed fallback. Do not remove that fallback before the new route is tested.

Input ACK latency and bandwidth estimation should be measured on that route as
well as Direct. None of these gathering fixes promises a particular RTT or
capture-to-display latency.
