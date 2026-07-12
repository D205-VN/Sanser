# Network modes

Sanser 2 exposes only `Auto`, `Direct`, and `Relay`.

The current desktop provides authenticated Native Direct on a reachable IPv4
route, normally the same LAN. It keeps signed discovery, WebRTC/libdatachannel,
and shared SNV2 framing disabled until their capability probes report verified
implementations.

The opt-in `p2p_v2` policy foundation and transient candidate signaling are
implemented, but they do not yet gather candidates or punch NATs at runtime.
See `docs/p2p-v2.md`; Native Direct remains the only working media route.

## Auto

Auto selects Native Direct when both devices advertise a reachable IPv4 route.
The control plane is ready to select WebRTC as a fallback, but that media engine
is not linked in the current desktop build.

## Direct

Direct disables TURN and uses only the authenticated native route. A firewall
or devices behind different NATs can block the connection.

## Relay

Relay requires TURN and WebRTC/libdatachannel, so the UI disables it until that
native engine is linked. Native Direct has no relay path.

## Discovery

Discovery messages contain a device identity, nonce/timestamp, TTL, sanitized capabilities, and signature. They are rate-limited, never authorize a session, do not treat any address range as a particular VPN, and stop when the application shuts down.

Use HTTPS/WSS and a restrictive origin allowlist outside loopback development. Configure short-lived TURN REST credentials rather than distributing a permanent TURN password.
