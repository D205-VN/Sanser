# iPhone → Windows WAN: no UDP received during Direct checks

The user reproduced this while an iPhone was connected to a Windows host on a
different network. The Mac project machine was not the viewer. The connected
iPhone reports Sanser 2.3.6, build 20314. No app was stopped or replaced.

## Evidence from the active iPhone session

| Observation | Value |
| --- | --- |
| Selected transport | Relay after Direct timeout |
| Network | Wi-Fi, IPv4 available, IPv6 unavailable |
| Failure stage | `check` |
| STUN succeeded | true |
| Router gateway available | true |
| Port mapping succeeded | false |
| Local / remote candidates | 4 / 3 |
| Local / remote public candidates | 1 / 1 |
| Peer acknowledged candidates | true |
| Candidate pairs checked | 12 |
| Successful local UDP send calls / send errors | 408 / 0 |
| UDP datagrams received by connectivity checker | 0 |
| Authenticated replies / verified pairs / nominations | 0 / 0 / 0 |
| Timed out | true |

These counters establish that gathering and candidate exchange completed, but
the iPhone checker received no datagram before its deadline. Successful send
calls mean the local OS accepted the datagrams; they do not establish that the
Windows host received them. Zero received datagrams does not establish whether
Windows ever started its own checks, whether an endpoint was unsuitable for the
peer, or whether filtering/NAT affected either direction. It does not prove a
credential, pair-hash, nomination, decoder, or 5 GHz radio fault.

STUN success demonstrates communication with the STUN service, not connectivity
to the Windows peer. Port mapping failure and a usable gateway do not by
themselves diagnose CGNAT or symmetric NAT. See
[RFC 8489](https://www.rfc-editor.org/rfc/rfc8489.html) and
[RFC 5128](https://www.rfc-editor.org/rfc/rfc5128.html).

## Code review and validation

Both clients use `sanser-p2p` authenticated SNP2 checks, session/device-bound keys,
and the same candidate-pair IDs. The iPhone observes a 15 second check deadline;
desktop observes 6 seconds. The mismatch is a possible timing consideration, not
a demonstrated cause for this session. Neither deadline was changed.

The nine existing `punch::tests` passed locally, including missed initial probe
bursts, authenticated peer-reflexive endpoint discovery, pair agreement, timeout,
bad authentication and pair-mismatch diagnostics. Loopback success does not
establish traversal of the actual two networks.

The desktop command currently calls `check_connectivity_measured`, discarding
the `ProbeDiagnostics` observer on failure. Consequently, its current generic
failure message cannot prove whether Windows received a probe. A future
diagnostics change should preserve those bounded counters without endpoints,
credentials or packet content. No runtime change was made in this investigation.

## Next evidence required

Read the matching Windows Diagnostics events for gathering, received candidates,
candidate acknowledgement, connectivity-check start and fallback. If Windows
starts its checks, capture per-side send/receive counters or a narrowly scoped
packet trace before attributing the loss to a firewall/router. Inspect existing
Windows Firewall rules for the actual `sanser.exe` path and active profile;
explicit block rules can override allow rules. Do not disable the firewall.
[Microsoft's rule precedence reference](https://learn.microsoft.com/en-us/windows/security/operating-system-security/network-security/windows-firewall/rules).

A subsequent A/B test can keep the Windows host unchanged and change only the
iPhone network, but that requires a new connection. It was not performed during
the user's active session. No claim that 2.3.7 resolves this NAT failure was made.
