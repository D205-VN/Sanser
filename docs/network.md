# Network modes

Sanser 2 exposes only `Auto`, `Direct`, and `Relay`.

This document defines the route-selection contract. The current desktop keeps
signed LAN discovery, WebRTC/libdatachannel, and native SNV2 routes disabled
until their runtime capability probes report a verified implementation.

## Auto

Auto evaluates signed LAN discovery and private routes first, then ICE host/server-reflexive candidates, TURN/UDP, TURN/TCP, and TURN/TLS. SNV2 is eligible only for an authenticated direct route. If its direct probe fails, the session negotiator falls back to WebRTC without leaving an accepted native session orphaned.

## Direct

Direct disables TURN. LAN/private/public routes and STUN-derived candidates are allowed. The UI reports NAT/firewall failure instead of silently waiting on an impossible relay.

## Relay

Relay requires TURN and uses WebRTC/libdatachannel. It prefers UDP, then TCP/TLS. Native SNV2 is not selected because it has no relay path.

## Discovery

Discovery messages contain a device identity, nonce/timestamp, TTL, sanitized capabilities, and signature. They are rate-limited, never authorize a session, do not treat any address range as a particular VPN, and stop when the application shuts down.

Use HTTPS/WSS and a restrictive origin allowlist outside loopback development. Configure short-lived TURN REST credentials rather than distributing a permanent TURN password.
