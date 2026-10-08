# UDP relay and delivery feedback

Sanser now attempts an authenticated UDP relay after Direct cannot establish a
route. `/api/v2/relay/udp` is a WSS **control** channel for allocation, authorization
and peer lifecycle. Media and native input go through UDP once **both** peers have
confirmed their receiving addresses. If the server does not implement or enable
this endpoint, or UDP negotiation exceeds five seconds, desktop falls back to the
existing encrypted WSS bridge. Direct-only mode still does not use either relay.

## Enable on a reachable server

Configure both variables on the same Sanser server process:

```dotenv
RELAY_UDP_BIND=0.0.0.0:50001
RELAY_UDP_PUBLIC=relay.example.com:50001
```

The advertised hostname/IP and UDP port must reach that process. HTTP ingress or
an HTTPS tunnel alone is insufficient. For LAN testing on a Mac, advertise its
LAN address and allow inbound UDP on that port. Serving devices outside that LAN
requires a reachable public UDP endpoint (port forwarding, suitable public IPv6,
or a server/network that exposes UDP). These code changes do **not** move the
existing Render service onto the Mac or configure a public endpoint automatically.
Keep both variables unset on deployments without UDP ingress.

Use updated desktop builds on both machines. Diagnostics logs the selected
`UDP datagrams` or `WSS/TCP fallback`; saved connection reports include
`relayTransport`. This reports the negotiated transport, not an inferred route.

## Packet and lifecycle rules

* Allocation requires account authentication, device ownership and an accepted
  native session; authorization is checked again every 20 seconds.
* A random allocation key authenticates each datagram with HMAC-SHA256. A
  challenge proves receipt at an address before forwarding to it. Allocation
  credentials are delivered over the authenticated WSS control connection.
* Video/input remain end-to-end encrypted inside the relay envelope. The relay
  sees allocation identity and packet length, not the native media or input.
* Both envelope and end-to-end cipher accept bounded reordering while rejecting
  replays. A lost packet does not hold later packets behind TCP retransmission.
* Media writes do not build an application-level send queue. A saturated UDP
  socket drops packets, leaving native ACK/repair/congestion logic to recover.
* Lease is 60 seconds, refreshed by authenticated control liveness. Replacement
  allocations invalidate old keys and notify the other peer. UDP negotiation or
  session failure is visible; it is not presented as a successful desktop stream.

This first UDP relay uses ordinary UDP, not QUIC. WSS fallback still has TCP
head-of-line behavior. UDP does not guarantee packet delivery or zero queueing in
the operating system/router, and no Internet latency target is guaranteed.

## Input and bitrate changes

Mac input coalesces gamepad snapshots in both Auto and Ultra. Connected snapshots
are not retransmitted after they become stale; disconnect and keyboard/button
batches remain acknowledged. Input retry timers follow a fresh measured network
RTT (bounded 8–250 ms, 40 ms with no fresh sample). Exhausted UDP retries enqueue
an acknowledged input reset in either profile, to avoid leaving a key held.

The Windows sender receives the Mac's unique accepted video payload byte count
and measurement duration before decode. It uses that delivered rate together
with offered rate, RTT trend and socket work to adjust the bitrate under sustained
pressure. An idle desktop cannot impose an artificially low bandwidth ceiling;
a stable high RTT or jitter alone does not cut bitrate. Each reduction is bounded
and only one can occur per statistics window. Network feedback does not lower FPS.
This is a conservative delivery estimator, not a full probing/BBR implementation.

## Verification and limits

Automated tests cover challenge/address binding, authentication, expiry,
replacement, replay rejection, out-of-order delivery, lost registration retry,
legacy endpoint fallback, and native input arriving after a dropped video packet.
Native regression traces cover idle traffic, sustained delivery deficit, RTT-based
input retry and keeping FPS stable under network pressure.

Real two-device Windows/Mac WAN tests and GPU performance measurements are still
required. The tests establish protocol behavior, not a measured 5–15 ms desktop.
