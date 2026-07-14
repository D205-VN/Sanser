# P2P v2 rollout

Sanser is migrating from the reachable-IPv4 Native Direct path toward one
authenticated UDP component for discovery, connectivity checks and, eventually,
the shared SNV2 media wire. Version 2.0.7 adds automatic direct-to-relay
fallback around the native engine. Direct UDP still cannot be guaranteed
through every NAT, so Auto switches both peers to an encrypted WSS/443 route.

The design follows the STUN Binding model in
[RFC 8489](https://www.rfc-editor.org/rfc/rfc8489) and the candidate/checking
principles in [RFC 8445](https://www.rfc-editor.org/rfc/rfc8445), without adding
a second WebRTC media pipeline.

## Implemented runtime foundation

- `crates/sanser-p2p` provides a bounded candidate schema, deterministic
  candidate and pair priority, deduplication, STUN Binding, authenticated UDP
  connectivity probes, endpoint locking and a strict P2P state machine.
- `sanser-network/p2p_v2` exposes those primitives behind an opt-in Cargo
  feature. Legacy route policy remains the default.
- `native/protocol` contains a separate bounded C++ STUN Binding wire codec with
  RFC 5769 IPv4/IPv6 vectors. Desktop candidate gathering currently uses the
  Rust implementation in `sanser-p2p`; the C++ codec is not the owner of the
  runtime gathering socket.
- The authenticated signaling WebSocket accepts `p2p.candidates`,
  `p2p.candidatesAck` and the compatibility `p2p.gatheringComplete` receipt only
  for an accepted native session and only between that session's two devices.
- Candidate metadata is validated with the shared crate, bounded to 16 entries
  per message, 64 per peer and 128 per session, retained in memory for at most
  five inactive minutes, and never written to Neon or the durable event log.
- Identical same-generation retries are idempotent and forwarded again without
  consuming quota. Reusing a candidate ID for a different endpoint or an
  endpoint under a different ID is rejected.
- Desktop coordination retries candidate delivery with bounded backoff until a
  matching-generation receipt arrives. It starts connectivity checks only after
  it has both remote candidates and acknowledgement of its own batch.
- Connectivity probes derive their authentication key from the accepted
  session UUID and opaque session credential, verify the expected peer device,
  and accept packets only from the endpoint authorized by the candidate pair.
- Rust and portable C++ now share the fixed 16-byte cumulative ACK plus 64-bit
  SACK-mask contract and golden semantics. It remains a wire primitive only;
  the sidecars do not emit, consume or retry these packets yet.

Candidate batches use this schema:

```json
{
  "type": "p2p.candidates",
  "sessionId": "SESSION_UUID",
  "targetDeviceId": "DEVICE_UUID",
  "payload": {
    "generation": 1,
    "candidates": [
      {
        "id": "srflx-1",
        "type": "serverReflexive",
        "address": "203.0.114.10",
        "port": 43892,
        "protocol": "udp",
        "interfaceIndex": 4,
        "mappingProtocol": "none",
        "priority": 1509949695,
        "foundation": "srflx-v4-4"
      }
    ]
  }
}
```

The address above is illustrative. Production gathering supplies the actual
validated endpoint, while diagnostics mask public addresses by default.

Current clients acknowledge a received batch with:

```json
{
  "type": "p2p.candidatesAck",
  "sessionId": "SESSION_UUID",
  "targetDeviceId": "DEVICE_UUID",
  "payload": { "generation": 1 }
}
```

The desktop also sends `p2p.gatheringComplete` with the same generation for
compatibility with servers deployed before the explicit acknowledgement was
added. Current servers validate and forward both receipts; acknowledgement
metadata is not persisted.

## Socket lifecycle and engine handoff

For one negotiation attempt, the Tauri backend reserves an IPv4 socket and,
when the OS exposes a usable global address, an IPv6-only socket on the same
local port. Candidate gathering and authenticated checks reuse those exact
sockets. Both peers try compatible global IPv6 pairs first, then automatically
fall back to IPv4 host/STUN/UPnP/manual candidates. A successful check leaves
the selected socket reserved instead of dropping it immediately.

Connectivity checks also learn one authenticated peer-reflexive endpoint per
candidate pair. This covers endpoint-dependent NATs that rewrite a peer's UDP
source port differently from the port observed by STUN. The learned endpoint is
accepted only after the probe HMAC, session ID and device identity match, and is
then pinned for the rest of the attempt; unauthenticated endpoint migration is
never allowed.

When `launch_engine` is called, Tauri verifies that the requested engine port is
the selected socket's port, releases the reservation at the last possible
moment, and starts the native sidecar so it can bind that same local port. A
failed or cancelled negotiation calls `p2p_stop` and releases the reservation.

This is a port handoff, not an operating-system socket-handle transfer: the
sidecar receives the selected endpoint and port, not the Rust socket itself.
Closing and rebinding the same port narrows the race but cannot prove that every
OS/NAT will preserve the external mapping. Passing ownership of the live socket
to the native engine, or moving gathering and checks into that engine, remains a
production-hardening item.

The Windows host reserves a configurable fixed UDP port (`50000` by default).
UPnP IGD discovery maps that exact port with a 1.5-second cap. If the router has
a manual same-port forwarding rule, the coordinator also advertises a bounded
manual candidate using the STUN-discovered public address and fixed port. PCP
and NAT-PMP remain disabled until their gateway detection and lease lifecycle
are production-ready.

## Retry and failure lifecycle

- Candidate exchange has a 20-second overall deadline. Candidate messages are
  retried with backoff up to two seconds while the peer socket is not ready or a
  receipt has not arrived.
- The Windows host waits for `requesterReadyAt` before starting negotiation, so
  the macOS requester can announce readiness first. Signaling retry covers the
  remaining WebSocket-registration race.
- A failed P2P attempt no longer disconnects the already accepted server
  session. Both peers retry up to three times with a bounded delay, then expose
  **Retry connection** / **Retry native stream** without entering a tight loop.
- Each requester retry publishes a new `requesterReadyAt` negotiation generation.
  Credentials are derived from that generation rather than the original accept
  time, and the Windows host resets its bounded retry counter when the generation
  changes. An expired credential is therefore replaced without recreating the
  accepted session.
- A manual stop or disconnect still ends the session and stops the native
  engine. A successful route selection is not reported as a connected media
  stream until engine launch succeeds.

## Remaining native transport hardening

1. Add explicit start-of-frame or fragment-index/count semantics. A receiver
   must never complete a frame when its first fragment is missing.
2. Wire the defined ACK/SACK payload into ordered reliable input delivery,
   bounded retry and fail-safe release of held keys/buttons before replacing
   the TCP control path.
3. Define the video, audio, input and feedback payload schemas carried by SNV2.
4. Add a lightweight native receive dispatcher and bounded per-lane queues so
   video decode cannot block audio, keepalive or input acknowledgements.
5. Add one native TX scheduler with pacing; capture/audio workers must not burst
   independently onto a shared socket.
6. Replace the close/rebind port handoff with live native socket ownership, then
   carry the selected route through keepalive, migration and rekey without
   falling back to unauthenticated endpoint changes.
7. Complete Windows CI plus repeated real Windows-to-macOS hardware tests for
   direct and relay routes before treating the transport as release-verified.

## Encrypted relay fallback

The P2P v2 path does not require Tailscale. It first gathers global IPv6 plus
IPv4 host, STUN, UPnP and fixed manual-forward candidates. In Auto mode, a
failed direct check opens an outbound WSS/443 connection from both desktops to
the authenticated Sanser relay. A local Rust bridge moves native UDP datagrams
through that socket without routing packet payloads through the webview.

Relay frames use XChaCha20-Poly1305 with a key derived from the ephemeral native
session credential. The session ID and bounded frame header are authenticated
as additional data, each bridge uses a random 128-bit nonce prefix, and receive
sequence numbers reject replay. The server authorizes the account, accepted
session and exact requester/host device pair, then forwards ciphertext only.
Direct mode never falls back; Relay mode skips direct probing. Auto currently
switches direct to relay during setup; seamless live route migration after an
engine has started remains future hardening.
