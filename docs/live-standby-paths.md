# Part 2: continuously measured Direct standby paths

Development on `Version-2.0.0` after the Opus/PLC work. This is not in the
published 2.2.15 installers and does not deploy a public UDP relay.

## Runtime integration

After authenticated candidate nomination, the desktop retains the **original
IPv4 UDP socket** used for gathering, STUN and punching. Previously it released
that reservation immediately before native startup. `direct.rs` now gives the
native engine a fixed loopback UDP peer and forwards opaque datagrams through
that retained external socket. All session keys, packet sequences, video/audio
formats, decoder state and input ACK/retry rules stay with the native engine.
There is no webview media callback, video decode/re-encode, frame queue, TCP
transport or new Internet server in this router. It uses nonblocking datagram
writes; local overload drops packets rather than accumulating a media backlog.
The extra local UDP hop and OS buffers still have cost; real-machine A/B latency
and throughput validation are required before claiming a performance improvement.

Both Legacy Mac controlling Windows and SNV2 desktop directions use this path
when launched from an authenticated P2P selection. Manual launches without a
reserved selection and existing Relay transports retain their existing behavior.
Group hosts own one router per viewer. Removing a viewer or dropping its engine
releases that router; other viewer sessions remain independent. A native Stop
can still send its final packets before the owning router is dropped.

## Measurement and selection

- Probe active and advertised alternatives continuously: up to four requests
  every 500 ms, at most 64 outstanding transactions. Retain up to 17 initial
  unique endpoints (selected plus 16 advertised), with room for learned NAT
  ports within a hard cap of 20. The wildcard socket uses OS routing for each
  destination; this does not pin a physical NIC or invent another ISP path.
- `SRP1` has a fixed 96-byte HMAC-SHA256 authenticated envelope, bound to the
  session and expected peer device. It is distinct from nomination and media.
  Measure RTT with the sender's monotonic clock, never subtract remote clocks.
- Match transaction and exact return endpoint; a reply counts once. Probe
  timeout is 1.5 s. Keep eight outcomes per endpoint, expiring after eight
  seconds. Score median RTT + twice median absolute deviation + loss percentage
  points. These are small-probe loss measurements, not video packet loss.
- Require at least three fresh successful samples, <=20% probe loss, and three
  successive confirmations using **new** samples. For a responding active path,
  require score improvement >=15 ms and >=25%. Re-evaluating one fast sample
  cannot accumulate votes. Do not switch during the first three seconds.
  Fresh-response eligibility includes a complete bounded probe round (up to
  2.5 seconds for 20 endpoints); validation does not expire between rounds.
  Finish confirming a still-qualified alternative instead of oscillating
  between equally fast alternatives on tiny score differences.
- Only the controlling peer proposes a better route. A switch must present an
  unexpired random return-path challenge from that exact endpoint. The follower
  also needs its own successful reverse probe; the leader changes its outgoing
  endpoint only after the authenticated matching switch ACK. Requests/replies
  are retried with a two-second bound and are idempotent.
- Preserve authenticated, recently verified inbound endpoints during transition
  so a delayed ACK does not interrupt incoming media. No endpoint is accepted
  as a standby media source merely because it sent an unauthenticated packet.
  Native media authentication and replay protection remain in force.
- Apply a 15-second cooldown for normal optimization and failed switch attempts.
  Sustained loss of the active probe path can bypass the optimization cooldown.
  A follower can fail back only to its previous, freshly measured responding
  path when its active path loses responses. This is a bounded failback, not a
  full new candidate gathering or an arbitrary unauthenticated route change.

With an old peer, unmatched probe packets are ignored by its native engine;
ordinary media stays on the original selected route. No verified probe response
means no automatic in-flight switch. Failure to find a faster route is not an
error and cannot justify claiming that Internet RTT has become 10 ms.

## Diagnostics and lifecycle

Connection details shows the actual current endpoint, each measured standby,
median RTT, jitter, probe loss and switch count. Events are scoped to the P2P
attempt. Updates become historical after five seconds without a new snapshot;
missing/insufficient measurements are unknown, not zero. Numeric-only
`DIRECT_PATH_HEALTH` samples are added to bounded local reports. No probe key,
challenge, credential, native payload or screen/input/audio content is logged.

While a same-session router is reporting, the old latency-only reconnect rule
cannot close the native window to race the same candidates again. Explicit peer
loss/control silence still uses existing bounded authorized reconnect; manual
Reconnect and Disconnect retain their existing semantics. The router cannot
create a session, grant input/audio permission or contact a non-candidate IP.

## Validation and remaining scope

Synthetic Rust tests exercise a delayed ~200 ms path and a fast alternative to
the same peer, flowing opaque media during measurement and switch, then video,
input and audio payloads in both directions. Source ports and native sockets
remain unchanged. Other tests cover HMAC/peer/session rejection, duplicate/stale
responses, small/unstable/lossy improvements, cooldown, failed switch ACK,
return-path challenge replay, verified failback, old peers, original port
reservation, UI attempt ownership and reconnect suppression. No test captures
a real screen, plays audio or injects physical keyboard/mouse events.

Still needed: real Windows/Mac tests over LAN, WAN, VPN/ZeroTier and changed NAT;
loaded throughput/latency A/B; active network loss and sleep/resume. This version
probes already advertised IPv4 destinations. It does not re-gather candidates
mid-session, repair an expired router mapping, complete IPv6 media, migrate
Direct to/from WSS/UDP Relay, select relay locations, or make a public UDP
service available without public ingress. Such transitions need separate
negotiation, server availability and validation.
