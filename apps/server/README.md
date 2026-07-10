# Sanser server 2.0.0

Rust signaling and account service for Sanser protocol v2. It carries account,
presence, session negotiation, SDP/ICE signaling and notifications only. Video,
audio and realtime input never pass through this service or the database.

## Run locally with SQLite

From the repository root:

```bash
STORAGE_MODE=local SQLITE_PATH=./data/sanser.db cargo run -p sanser-server
```

The parent directory is created automatically. Local SQLite uses WAL mode, a
bounded connection pool and versioned SQLx migrations. No PostgreSQL installation
is needed for LAN/local mode.

## Run shared mode with PostgreSQL

Set `DATABASE_URL` to a PostgreSQL URL; it takes precedence over `SQLITE_PATH`:

```bash
STORAGE_MODE=shared \
DATABASE_URL=postgresql://USER:PASSWORD@HOST:5432/sanser \
ALLOWED_ORIGINS=https://app.example.com \
cargo run -p sanser-server
```

Migrations are applied at startup. The readiness endpoint returns `503` until
the database can answer a bounded `SELECT 1` query.

## TURN without Tailscale

For Internet relay, configure coturn (or a compatible TURN REST service) and use
a shared secret. The API derives a user-scoped HMAC-SHA1 credential with a short
expiry; the shared secret is never returned or logged.

```text
NETWORK_MODE=relay
TURN_URLS=turn:relay.example.com:3478?transport=udp,turns:relay.example.com:5349
TURN_SHARED_SECRET=replace-with-coturn-static-auth-secret
TURN_CREDENTIAL_TTL_SECONDS=3600
```

`NETWORK_MODE=auto` returns STUN plus TURN and lets the native networking layer
try direct ICE before relay. `direct` does not require TURN. Static
`TURN_USERNAME`/`TURN_CREDENTIAL` remains supported for small self-hosted setups,
but `TURN_SHARED_SECRET` is preferred because its client credentials expire.

## API

All authenticated REST calls use `Authorization: Bearer <access token>`.
Opaque access and refresh tokens are stored only as SHA-256 digests, have
independent expiry, support rotation/revocation and are never logged.

```text
POST   /api/v2/auth/register
POST   /api/v2/auth/login
POST   /api/v2/auth/logout
POST   /api/v2/auth/refresh
GET    /api/v2/account
POST   /api/v2/account/password
GET    /api/v2/account/sessions
DELETE /api/v2/account/sessions/:id
GET    /api/v2/devices
POST   /api/v2/devices/register
POST   /api/v2/devices/heartbeat
PATCH  /api/v2/devices/:id
DELETE /api/v2/devices/:id
POST   /api/v2/sessions
GET    /api/v2/sessions/:id
POST   /api/v2/sessions/:id/accept
POST   /api/v2/sessions/:id/reject
POST   /api/v2/sessions/:id/disconnect
GET    /api/v2/network/ice
GET    /api/v2/health
GET    /api/v2/readiness
WS     /api/v2/events
WS     /api/v2/signaling?deviceId=<uuid>
```

Browser WebSocket clients send subprotocols `sanser-v2` and
`bearer.<access-token>`. Native clients may instead provide the normal
`Authorization` header. Signaling messages are JSON text, capped at 64 KiB and
placed into a bounded per-device queue. A sender can only signal the other device
of an accepted session:

```json
{
  "sessionId": "uuid",
  "targetDeviceId": "uuid",
  "type": "iceCandidate",
  "payload": { "candidate": "..." }
}
```

Supported types are `offer`, `answer`, `iceCandidate`, `renegotiate` and
`connectionState`. Event notifications are persisted briefly and replayed after
reconnect; use `?since=<unix-seconds>` when a client has a checkpoint.

Every HTTP response includes `X-Request-ID`. JSON errors use
`{"error":{"code":"...","message":"..."}}`. CORS is an explicit allowlist,
request bodies and WebSocket frames are bounded, all state-changing routes check
account ownership, and graceful shutdown drains HTTP before closing the pool.

## Verification

```bash
cargo fmt --check --manifest-path apps/server/Cargo.toml
cargo clippy --manifest-path apps/server/Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path apps/server/Cargo.toml
```

The SQLite integration suite covers migration/readiness, register/login/logout,
access expiry, refresh rotation, digest-only token storage, devices, heartbeat,
session authorization/state transitions, ICE credentials, request IDs, invalid
payloads, rate limiting and cleanup. PostgreSQL uses the same SQLx `Any` queries
and migration; run the service against a disposable PostgreSQL database as part
of deployment verification.
