# P2P v2 rollout

Sanser is migrating from the current reachable-IPv4 Native Direct path to one
authenticated SNV2 UDP component that can be checked across LAN, IPv6, mapped
ports and STUN-discovered routes. The rollout is intentionally capability-gated:
the existing LAN path remains available until its replacement passes native
interoperability and impairment tests.

The design follows the STUN Binding model in
[RFC 8489](https://www.rfc-editor.org/rfc/rfc8489) and the candidate/checking
principles in [RFC 8445](https://www.rfc-editor.org/rfc/rfc8445), without adding
a second WebRTC media pipeline.

## Implemented foundation

- `crates/sanser-p2p` provides a bounded candidate schema, deterministic
  candidate priority, deduplication, verified-route scoring and a strict P2P
  state machine.
- `sanser-network/p2p_v2` exposes those primitives behind an opt-in Cargo
  feature. Legacy route policy remains the default.
- `native/protocol` contains a bounded STUN Binding wire codec with RFC 5769
  IPv4/IPv6 vectors. It deliberately performs no socket I/O yet.
- The authenticated signaling WebSocket accepts `p2p.candidates` and
  `p2p.gatheringComplete` only for an accepted native session and only between
  that session's two devices.
- Candidate metadata is validated with the shared crate, bounded to 16 entries
  per message, 64 per peer and 128 per session, retained in memory for at most
  five inactive minutes, and never written to Neon or the durable event log.
- Native capability probes remain honest: `nativeDirect=true` describes the
  current three-channel LAN path; `nativeSnv2=false` remains set until the
  shared SNV2 wire is used end to end.
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

## Socket ownership decision

The native sidecar owns the network socket. Tauri manages session lifecycle and
relays candidate metadata over authenticated local IPC; it must not open a
separate STUN socket or proxy realtime media. This preserves the NAT mapping
created for the exact UDP socket that later carries SNV2.

During IPv4/IPv6 racing an implementation may temporarily need one socket per
address family. The invariant is one UDP component and one selected five-tuple,
not an unsafe assumption that every operating system provides identical
dual-stack behavior.

## Required gates before runtime STUN and hole punching

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
6. Run STUN Binding transactions and connectivity probes from that same native
   socket, then add nomination, endpoint locking, keepalive and rekey.
7. Pass Rust/C++ golden vectors, loss/reorder/replay tests, Windows CI and a real
   Windows-to-macOS hardware test before setting `nativeSnv2=true`.

## No-relay limitation

The target P2P v2 policy does not carry media through the Sanser server and does
not require Tailscale. PCP, NAT-PMP, UPnP and UDP hole punching can improve
direct-connect success, but a no-TURN product cannot guarantee a connection
through every symmetric NAT, CGNAT, double-NAT or restrictive firewall. Failure
must be explicit, with manual port forwarding offered only through the same
authenticated session and encrypted SNV2 handshake.
