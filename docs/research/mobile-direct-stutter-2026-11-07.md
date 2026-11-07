# iPhone Direct stutter investigation — 2026-11-07

Reported setup: iPhone viewing/controlling a Mac on the same Wi-Fi. Reported
control RTT varies from 21 to 100 ms, received video from 0 to 33 FPS, with
increasing NACK (812–834) and Opus concealment (3100–3811) counters. These are
user observations, not a synchronized capture of packet loss or display latency.

## Changes in local iPhone build 20306

- Video repair can use authenticated same-transport control RTT before the first
  reliable keyboard/button ACK. Previously its retry interval remained 100 ms
  in pointer-only or view-only sessions. Unknown timing retains that default;
  measured timing uses RTT plus headroom, bounded to 20–200 ms. Input delivery
  deadlines and reliable ACK behavior remain separate.
- Both sockets in the Rust Direct bridge receive best-effort burst-buffer
  tuning. Failed large requests fall back to smaller supported sizes; existing
  larger buffers are retained. Receive target is 512 KiB and send target 64 KiB.
  Router snapshots include actual kernel-reported sizes. The native receiver
  also retries smaller receive-buffer requests if its 4 MiB request fails.
  This adds no application playout timer and does not change route identity,
  media authentication, FEC framing or source-port ownership.
- Mobile receives the native health snapshot's monotonic sample time. FPS and
  counter rates use that time rather than Flutter callback arrival time.
  Re-reading a snapshot no longer fabricates a zero-FPS sample; a genuinely new
  snapshot without frame progress still reports zero. NACK and concealed-audio
  rates are shown per second, while cumulative counters remain in diagnostics.
  Received-frame FPS is not a claim of rendered/displayed FPS.

## Verification

- Flutter analysis: no issues. Full mobile suite: 139 passed, including snapshot
  repeats, real zero-progress intervals and per-second repair/concealment rates.
- Rust P2P/mobile bridge: 99 passed. The additional constrained-buffer burst
  check starts with 8 KiB buffers and verifies binding plus intact delivery.
- Native input-engine, rate-control and socket-recovery checks: 3 passed.
- iPhoneOS ARM64 native build and Release/AOT app build 20306 completed.

## Physical acceptance still required

Reconnect iPhone to the same Mac and compare active scrolling/window motion
over at least 60 seconds with the same bitrate and sound settings. Record RTT,
received FPS, NACK/s and concealment/s together. An idle desktop can legitimately
produce few frames. The existing cumulative NACK/concealment totals alone cannot
identify current loss rate or establish that Wi-Fi or decoder scheduling is the
bottleneck. Neither a fixed LAN RTT nor elimination of audible breakup is
certified by these automated checks. The Opus playout algorithm is unchanged in
this increment.

## iPhone to Windows on a different Wi-Fi — build 20307

Mobile gathering previously disabled router mapping. iOS also returns no
gateway from the desktop command/proc-based gateway reader. The native plugin
now takes an IPv4 gateway from the current Wi-Fi/Ethernet `NWPath`, passes it
directly to the shared gatherer, and enables authenticated-path discovery with
unicast PCP/NAT-PMP mapping attempts. iOS does not depend on multicast UPnP
discovery. The gateway is never inferred from a subnet or supplied by signaling.
Mapping and STUN retain their existing bounded gathering timeout; a mapping
alone cannot nominate a route without the normal authenticated UDP checks.

Native gateway availability, mapping/STUN results and public-candidate counts
are retained in the mobile diagnostic report, without raw endpoints. A mapping
success followed by UDP failure is distinguished from STUN/mapping failure.
Synthetic mobile gather tests explicitly disable mapping to avoid changing a
real router during tests. Invalid native gateway targets are rejected early.

Correction to an earlier explanation: desktop gathers IPv6 candidates, but its
current media connectivity path explicitly filters to IPv4. IPv6 gathering is
not evidence of working IPv6 media Direct on desktop or mobile. This increment
does not claim to complete IPv6 media.

Verification: 140 Flutter tests and analysis pass; 94 shared P2P and seven
mobile C-ABI/bridge tests pass. The final diagnostic fixtures also pass. ARM64
iPhone native compilation and Release/AOT build 20307 complete. Real router
PCP/NAT-PMP acceptance and a successful WAN iPhone-to-Windows session still
need a physical test; a router may reject mapping or UDP traversal, in which
case Relay remains necessary.

## Authenticated UDP check evidence — local build 20308

The user confirmed that “5G” means Wi-Fi 5 GHz, not cellular 5G. The app
reports the native network category (Wi-Fi/cellular/Ethernet); it does not
infer radio band from an SSID or claim that NWPath identifies 2.4/5 GHz.
The reported WAN session still falls back to Relay after build 20307.

Connectivity checks now retain numeric counters for candidate pairs, accepted
send calls, send errors, received packets, authenticated packets, matching
session/peer identities, matching pairs, valid responses, two-way verified
pairs and nominations, plus selection/timeout flags. An accepted send call
does not prove delivery onto the wire. Normal HMAC, identity and candidate
pair validation remains enforced. No endpoints, SSIDs or credentials are
included in these new diagnostics.

The mobile bridge exposes the final check snapshot after failure as well as
success. Flutter retrieves it before closing a failed attempt and distinguishes
no UDP response from authentication/pair mismatch and incomplete nomination.
Older engines that lack the snapshot operation retain the existing fallback.
Network metadata rejects stale revisions and retains only typed capability
fields. Socket buffer tuning also runs before the checks, not just streaming.

Verification: 147 Flutter tests pass; 95 shared P2P and seven mobile bridge
tests pass. Real UDP tests distinguish a silent peer, a wrong authentication
key and an authenticated wrong pair; the existing successful nomination test
also verifies the snapshot. Flutter analysis and ARM64 iPhone native and
Release/AOT build 20308 pass. None of these checks proves that the user's
WAN path now supports Direct: the next physical test must supply the new
failure reason/counters before assigning the cause to the app, router or NAT.
