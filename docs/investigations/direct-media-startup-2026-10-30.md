# Direct probe succeeds but media never reaches the Mac client

## Observed evidence

The local 2.2.26 report for 15:38:34 on October 30 shows an authenticated
nomination with four RTT samples, median 9.3788755 ms and probe loss 25%.
The client ends after approximately 21 seconds with `startup-timeout`.
Native startup counters are all zero: raw UDP, control UDP, video UDP, completed
packets and decoded frames. Authentication rejects and unexpected-peer counts
are also zero. Periodic standby snapshots have no successful live RTT samples.
Several earlier attempts that afternoon show the same missing-data pattern;
the 14:55 session did receive media before those failures.

This establishes failed media startup after nomination. It does not establish
whether the Windows host failed to start, the peer path stopped working, the
router discarded incoming packets or local forwarding failed. An advertised
virtual-interface candidate is not proof of the physical interface actually used.
No Windows host report was available, and its UI showed no error.

## Instrumentation

The existing Direct router keeps the original nominated external UDP socket
and forwards opaque packets between it and a fixed loopback native endpoint.
The new cumulative counters observe each boundary on both host and client:

| Counter | Meaning |
| --- | --- |
| `routerExternalReceived` | All datagrams read from the external socket |
| `routerExternalProbes` | Datagrams with the SRP1 monitor prefix, including invalid ones; not proof of authentication |
| `routerPeerPayloads` | Original non-FEC payloads admitted by the existing endpoint rule |
| `routerExternalRejected` | Non-SRP1 datagrams rejected by that endpoint rule |
| `routerNativeReceived` | Datagrams read from the reserved native loopback endpoint |
| `routerNativeRejected` | Datagrams read locally from any other endpoint |
| `routerToNative`, `routerToPeer` | Successful UDP send calls for native traffic in each direction |
| `routerToNativeDrops`, `routerToPeerDrops` | Failed send calls for that traffic |
| `routerExternalReceiveErrors`, `routerNativeReceiveErrors` | Receive errors, including the already tolerated reset/refused cases |
| `routerStopReason` | 0 running; 1 owner stop/drop; 2 external receive fault; 3 local receive fault |

`routerToNative` also includes FEC-recovered payload sends. Monitor and parity
transmissions retain their existing counters and are not counted as native
outgoing traffic. Successful send calls do not prove peer receipt, valid native
authentication, decoded video or a presented frame.

Snapshots emit at router start, every two seconds and immediately before return
on stop/error. `DIRECT_ROUTER_HEALTH` stores only allowlisted numeric values in
the automatic connection report; live connection details receive the same
counter object. Packet contents, tokens and addresses are not added to persisted
samples. Existing authentication, endpoint filtering and scheduling are unchanged.

## Reading the next failed session

- Native outgoing packets with successful peer sends but zero external reads
  indicate no return traffic reached that router. Compare the host's counters.
- External reads consisting only of SRP1 probes do not establish working media.
- Admitted payloads and successful native sends with zero native raw reception
  narrow the issue to the local socket handoff/engine boundary.
- Rejected-source, send-drop or receive-error counters identify their respective
  boundary; they do not by themselves identify NAT, firewall or OS as the cause.

Loopback regression tests exercise bidirectional opaque forwarding, probe-only
traffic, foreign external/local senders and the final short-session snapshot.
Report tests verify numeric filtering and that router traffic cannot invent
video or RTT measurements. Local validation passed all 98 desktop library tests
and all 91 P2P library tests, Clippy with warnings denied, formatting and
repository policy checks. This is diagnostic work, not a confirmed repair of
the user's two-machine startup failure. No new release is published here.
