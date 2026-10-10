# Startup, capture recovery and overlay candidates

## Changes

- Auto now polls incomplete fragment assemblies for repair, as Ultra already
  did. A missing middle fragment was previously first requested at assembly
  eviction (850 ms), after the host's 150 ms retransmit cache deadline. The
  receiver now requests repair after a 3 ms reordering allowance when a final
  fragment or later packet proves a gap. A stalled tail also triggers repair
  without another packet arriving. This does not change Auto's playout policy.
- Before the first decoded frame, received video triggers a keyframe request
  after one second, limited to once a second until decoding succeeds. The
  existing 20 second startup failure remains bounded. Requests still use the
  authenticated control path. Existing crypto-ready keyframe requests remain.
- Media generation changes clear incomplete assemblies and obsolete NACKs
  before timer-based repairs, including when no new video arrives.
- Reports retain numeric incomplete/missing fragment, NACK sent/recovered/
  expired, retransmit completion, authentication/replay and receive-drop data.
- Windows capture recovery releases invalid duplication before retrying. It
  retains a healthy D3D device across desktop switches, recreates the encoder
  if the capture device changes, and requests a fresh keyframe on recovery.
  The first recovered capture is kept even if the desktop then remains idle.
  Capture pauses keep the control session alive; authenticated pong telemetry
  lets the Mac show a temporary desktop-unavailable message over the last image.
  The message does not claim that every capture loss is UAC.
- Interface name classification recognizes feth/veth/VM adapters as virtual
  before checking Ethernet names. VPN and overlay candidates remain eligible.
  Existing measured probe scoring takes precedence over interface preference.
  Diagnostics exposes local advertised interface metadata when it can be
  matched by both index and address. This is not proof of the OS egress route;
  private addresses and ICE `host` type do not establish physical LAN routing.

## Validation

The native Mac latency test exercises missing middle/tail fragments, reordering,
repair before cache expiry, generation reset and startup recovery throttling in
Auto and Ultra. Rust tests check overlay classification/eligibility and numeric
report filtering. Windows builds validate the native capture code; they do not
simulate an interactive Secure Desktop.

Real Windows + Mac checks still required:

1. Connect with Auto, including a lossy path. Inspect `SNU1_STATS`:
   `incomplete`, `missingFragments`, `nackSent`, `nackRecovered`, `authRejected`.
   Export the connection report if the first image still fails.
2. While streaming, open a normal UAC prompt on Windows and dismiss it locally
   after 10 seconds. Repeat with an unchanged desktop. The session should stay
   open, show capture-unavailable status and resume with a fresh keyframe.
   Test lock/unlock and display changes separately; none proves full UAC control.
3. With ZeroTier enabled, verify that feth is labelled virtual and remains a
   candidate. Compare probe RTT and same-socket UDP echo RTT under media load.
   No interfaces are brought up/down or reconfigured by this change.

## Remaining work

This patch does not add a new READY negotiation, automatic UDP-relay-to-WSS
relaunch, live route migration, PCP/NAT-PMP, authoritative NAT-type detection,
or a privileged Windows service. It does not promise Direct through every NAT
or a particular real-world latency. Public UDP relay validation still requires
a reachable UDP server. Viewing/controlling Secure Desktop needs a separate
privileged architecture; this change restores ordinary desktop streaming.

DXGI desktop-switch recovery follows Microsoft's documented behavior:
[AcquireNextFrame](https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgioutputduplication-acquirenextframe).
