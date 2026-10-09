# Measured Direct route selection

The controlling peer collects paths for up to 500 ms after the first authenticated,
bidirectionally successful pair. This is connection setup time, not a media buffer.
All compatible pairs remain eligible during that window; the initial punch/retry
schedule continues for paths that have not succeeded yet.

A successful pair gets four extra probes, at least 60 ms apart, plus its initial
RTT proof. RTT uses a local monotonic send instant and the matching response;
duplicates, unknown transaction IDs and incorrect echoed timestamps do not become
samples or pin a response endpoint. Late initial-burst replies do not pollute the
five-sample measurement set. HMAC, session, peer identity and endpoint pinning
remain required. The wire protocol is unchanged.

Selection prefers pairs with at least three RTT samples, then minimizes:

```
score_ms = median_RTT + 2 * median_absolute_deviation
           + expired_loss_fraction * max(median_RTT, 50)
```

Only post-verification measurement probes that have expired (at least 100 ms and
twice the maximum observed RTT), or local measurement send failures, count toward
loss. Still-in-flight probes and missed startup probes do not. This small sample
is a route-selection signal, not a sustained-bandwidth or loss measurement.
Candidate priority, including advertised interface preference, breaks ties; there
is no fabricated physical-interface/metered-network cost. The IPv4 engine uses a
wildcard socket: multiple candidate IDs can describe the same actual path, and
this change does not bind independent sockets to each NIC.

The remaining timeout budget reserves three estimated round trips plus 350 ms for
nomination/commit/final and peer linger. If the window must shrink, or replies are
lost, selection can fall back to a verified pair with fewer than three samples;
Diagnostics labels that limited confidence. Only the controlling peer nominates;
the other peer follows the same pair rather than independently switching routes.
Both desktops should be updated: an older controlling peer can still nominate its
first success.

## Diagnostics and media comparison

Settings → Diagnostics → Connection details includes the latest selected pair:
local/remote advertised candidate types and endpoints, authenticated remote
endpoint (which can differ due to peer-reflexive NAT), probe count, median/min/max,
loss and reason. These addresses do not identify the physical NIC chosen by the
OS. Diagnostics export includes these endpoints; automatic bounded connection
reports retain numeric probe statistics and attempt identity, without endpoints.

After video is observed and a two-second warm-up passes, authenticated
`SNCONTROL_TIMING.wireEstimateMs` measurements are compared with the **locally
measured** selection median. Three consecutive samples above
`max(80 ms, 2 * probe median, probe median + 40 ms)` produce an
`elevated-after-media` warning in Diagnostics and the local connection report.
A gap over five seconds resets the streak. Fewer than three probe samples does
not produce a confident comparison. Events from earlier connection attempts are
ignored. A normal sample clears the current warning; there is no automatic route
migration or reconnect.

Probe RTT includes probe handling. Live Wire RTT subtracts instrumented app work
but still includes OS queues/scheduling. A large increase is evidence to investigate
media-path load, NAT/router behavior and OS/network queues, not proof of a specific
cause. Conversely, similarly high probe and live RTT do not prove a faster path
exists. An unrelated ICMP ping is not a baseline for this path.

## Verification

Automated UDP tests emulate a ~200 ms path that succeeds first and a ~14 ms path
that becomes available later. Other checks cover response replay/mismatches,
median outliers, expired versus in-flight probes, same-pair agreement, short
negotiation budgets, a late-starting peer and authenticated peer-reflexive ports.
Report/UI tests cover warm-up, sustained increases, stale samples/attempts and
warning deduplication.

On the two real computers: reconnect, record the selected candidate and probe
statistics, then capture Ctrl + Option + 8 after 10–20 seconds of desktop movement.
Compare **probe median**, **Live Wire RTT**, **Input ACK RTT** and the queue timings.
No Internet route or end-to-end latency target is claimed by the local tests.
