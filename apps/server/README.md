# Sanser server

Axum API, account service, device registry and WebSocket signaling for Sanser
2.0.0. The server is PostgreSQL-only and accepts only TLS Neon endpoints. It
does not contain a SQLite or local-database fallback.

## Neon configuration

Create a Neon database/branch and use its pooled connection string when
available. Keep the credential in `.env`, never in a tracked file:

```dotenv
DATABASE_URL=postgresql://USER:PASSWORD@EP-NAME-pooler.REGION.aws.neon.tech/DATABASE?sslmode=require
DATABASE_MAX_CONNECTIONS=10
DATABASE_MIN_CONNECTIONS=1
DATABASE_ACQUIRE_TIMEOUT_SECONDS=10
NATIVE_BASE_PORT=50000
SESSION_CREDENTIAL_TTL_SECONDS=60
SESSION_CREDENTIAL_KEY=BASE64_ENCODED_RANDOM_32_BYTE_OR_LONGER_KEY
```

`DATABASE_URL` must use `postgres://` or `postgresql://`, have a hostname ending
in `.neon.tech`, name one database, and contain exactly one
`sslmode=require`. URL-encode special characters in credentials. A non-Neon,
non-TLS, SQLite, or missing URL stops startup before any connection is opened.

The default pool keeps one connection warm, uses up to 10 connections, caches
prepared statements, and periodically retires connections. Lower the maximum
for small Neon plans or increase it only after checking the Neon connection
limit. Never log the complete connection string.

`SESSION_CREDENTIAL_KEY` is mandatory. Generate it independently of database,
TURN, token and discovery secrets (for example, from 32 random bytes encoded as
base64). It is held in zeroizing memory, never stored in PostgreSQL and never
returned. Rotating it invalidates native handoffs that have not started yet.

Start the API from the repository root:

```bash
npm run server:dev
```

SQLx applies `apps/server/migrations` on startup. Readiness reports failure
until PostgreSQL responds and migrations finish.

## Main routes

```text
GET    /api/v2/health
GET    /api/v2/readiness
POST   /api/v2/auth/register
POST   /api/v2/auth/login
POST   /api/v2/auth/refresh
POST   /api/v2/auth/logout
GET    /api/v2/account
GET    /api/v2/account/sessions
DELETE /api/v2/account/sessions/:id
POST   /api/v2/account/password
GET    /api/v2/devices
POST   /api/v2/devices/register
POST   /api/v2/devices/heartbeat
POST   /api/v2/devices/offline
PATCH  /api/v2/devices/:id
DELETE /api/v2/devices/:id
POST   /api/v2/sessions
GET    /api/v2/sessions/:id
GET    /api/v2/sessions/:id/credentials?deviceId=<uuid>
POST   /api/v2/sessions/:id/accept
POST   /api/v2/sessions/:id/reject
POST   /api/v2/sessions/:id/disconnect
GET    /api/v2/network/ice
WS     /api/v2/events
WS     /api/v2/signaling?deviceId=<uuid>
```

Access and refresh tokens are random opaque values; only SHA-256 digests are
stored. Passwords are Argon2id hashes. API queries use bound PostgreSQL
parameters. CORS uses the explicit `ALLOWED_ORIGINS` list and TURN credentials
can be generated from a shared secret.

The native credential route is available only while an account-owned session is
accepted with `selectedTransport=snv2`. `deviceId` must be one of that session's
two online, native-capable devices. The response contains the peer route,
`basePort`, an acceptance-anchored expiry and the same opaque HMAC-SHA256
`sessionToken` for both peers. Responses are `Cache-Control: no-store`. The
credential expires after 15–300 configured seconds; create a new session after
expiry. This is a secure handoff and does not advertise the unfinished native
SNV2 engine as available.

Before a client exits or signs out, it should call:

```http
POST /api/v2/devices/offline
Authorization: Bearer <access-token>
Content-Type: application/json

{"deviceId":"<uuid>"}
```

The endpoint returns the updated device, writes `online=false`,
`streaming=false` and `lastSeenAt` immediately, and publishes a
`device.presence` event when state changed. It is safe to call more than once.

## Tests

Unit tests do not need a database:

```bash
cargo test -p sanser-server --lib
```

Database-backed API tests are opt-in and read only `TEST_DATABASE_URL`—never
`DATABASE_URL`. The URL must pass the same Neon/TLS validation. Each test creates
a random `sanser_test_*` schema, sets it as its connection search path, runs the
migrations there, and drops it with `CASCADE` at the end:

```bash
TEST_DATABASE_URL='postgresql://...neon.tech/TEST_DATABASE?sslmode=require' \
  cargo test -p sanser-server --test api
```

Use a dedicated Neon test branch/database even with schema isolation. A process
kill or test panic can interrupt asynchronous cleanup, in which case stale
`sanser_test_*` schemas should be reviewed and removed manually.
