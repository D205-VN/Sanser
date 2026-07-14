# Sanser 2 security model

## Trust boundaries

The Svelte WebView, Tauri core, sidecars, remote peers, discovery datagrams,
signaling/relay server, and database are separate trust boundaries. Every
boundary validates length, type, identity, ownership, and lifetime.

## Required controls

- Argon2id password hashes with per-password salt.
- Short-lived access tokens and revocable refresh tokens; databases store digests.
- OS secure storage for local long-lived credentials.
- Authenticated `/api/v2/network/ice`; permanent optional TURN compatibility
  secrets are never returned.
- The WSS relay authorizes the exact accepted session and device pair, bounds
  queues/frame rates, and forwards only opaque end-to-end encrypted datagrams.
- CORS allowlist, request IDs, body limits, rate limits, parameterized SQL, and sanitized errors.
- SNV2 session authentication plus an XChaCha20-Poly1305 relay envelope with
  direction identity, nonces, replay rejection and bounded payloads.
- Tauri allowlist/capability files with no arbitrary shell or filesystem access.
- Sidecar executable and argument whitelists; no `shell=true`.
- Sanitized diagnostics/log exports.

The desktop accepts native capability only from a matching product/version probe.
Session credentials bind the selected requester, host and negotiation generation;
relay adds an outer authenticated encryption layer before packets leave Tauri.
Cross-platform hardware verification remains required for every release.

## Environment warning

`.env` is ignored and locally restricted to owner read/write. A database credential appeared in historical commits before the Sanser 2 migration. It must be rotated/revoked before release, then removed from Git history with a coordinated history rewrite. Deleting the current file is not sufficient.

## Internet deployment

Use HTTPS/WSS at the reverse proxy for any non-loopback server. HTTP is acceptable only for isolated local development. Configure restrictive `ALLOWED_ORIGINS`; do not use wildcard origins with credentials.
