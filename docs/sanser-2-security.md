# Sanser 2 security model

## Trust boundaries

The Svelte WebView, Tauri core, sidecars, remote peers, discovery datagrams, signaling server, databases, and TURN infrastructure are separate trust boundaries. Every boundary validates length, type, identity, ownership, and lifetime.

## Required controls

- Argon2id password hashes with per-password salt.
- Short-lived access tokens and revocable refresh tokens; databases store digests.
- OS secure storage for local long-lived credentials.
- Authenticated `/api/v2/network/ice`; permanent TURN secrets are never returned.
- TURN REST credentials derived with a short TTL when `TURN_SHARED_SECRET` is configured.
- CORS allowlist, request IDs, body limits, rate limits, parameterized SQL, and sanitized errors.
- SNV2 session authentication, direction keys, replay windows, endpoint verification, and bounded payloads.
- Tauri allowlist/capability files with no arbitrary shell or filesystem access.
- Sidecar executable and argument whitelists; no `shell=true`.
- Sanitized diagnostics/log exports.

The shared SNV2 library currently verifies its bounded header and authentication
primitives in isolation. The Windows and macOS media engines have not integrated
that shared handshake, session binding, packet framing, and replay protection
end to end, so their `nativeSnv2` capability must remain disabled until an
interoperability test passes.

## Environment warning

`.env` is ignored and locally restricted to owner read/write. A database credential appeared in historical commits before the Sanser 2 migration. It must be rotated/revoked before release, then removed from Git history with a coordinated history rewrite. Deleting the current file is not sufficient.

## Internet deployment

Use HTTPS/WSS at the reverse proxy for any non-loopback server. HTTP is acceptable only for isolated local development. Configure restrictive `ALLOWED_ORIGINS`; do not use wildcard origins with credentials.
