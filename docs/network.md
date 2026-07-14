# Network modes

Sanser 2 exposes only `Auto`, `Direct`, and `Relay`.

The desktop gathers IPv4/IPv6, STUN, UPnP and manual-forward candidates and
runs authenticated UDP connectivity checks. The native sidecar owns media and
input; WebRTC/libdatachannel is not part of the current media path.

## Auto

Auto tries Native Direct first and falls back to the authenticated Sanser WSS
relay when no UDP route can be established. Both endpoints make outbound
connections, so the fallback works through CGNAT and networks that block
unsolicited inbound traffic.

## Direct

Direct uses only the authenticated native UDP route. A firewall or devices
behind different NATs can block the connection, and no relay is attempted.

## Relay

Relay skips UDP traversal and carries native SNV2 datagrams through the Sanser
WSS/443 bridge. Frames are encrypted at the two desktop endpoints with the
ephemeral session credential. The relay forwarding path receives ciphertext
and never parses or decrypts video, audio or input packets.

## Discovery

Discovery messages contain a device identity, nonce/timestamp, TTL, sanitized capabilities, and signature. They are rate-limited, never authorize a session, do not treat any address range as a particular VPN, and stop when the application shuts down.

Use HTTPS/WSS and a restrictive origin allowlist outside loopback development.
Run only one relay-capable server instance unless relay session affinity or a
shared relay backplane is configured.
