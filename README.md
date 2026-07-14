# Sanser 2.0.7

Sanser is a low-latency remote desktop and remote game streaming platform. Version 2 is built as one Tauri application with a Svelte 5 interface, a Rust control plane, and native Windows/macOS media engines.

```text
Product:  Sanser 2.0.7
App ID:   com.sanser.desktop
Protocol: v2
Native:   Authenticated SNV2 media engines
```

## Capabilities

- Account, device and signaling data backed only by PostgreSQL on Neon.
- Local preferences stored as a bounded JSON file; secrets stay in Keychain/Credential Manager.
- Versioned `/api/v2` auth, devices, connection sessions, ICE and bounded WebSocket candidate signaling.
- Auto, Direct and Relay modes with native UDP plus an end-to-end encrypted WSS/443 fallback for CGNAT and blocked UDP.
- Short-lived access tokens, refresh-token rotation/revocation and Argon2id passwords.
- Typed Tauri commands and sidecar allowlists; Node.js is not a packaged runtime.
- Bounded SNV2 packet, input, audio, retransmission and diagnostic primitives.
- Windows C++20 host and Objective-C++20 macOS client migration paths.

Platform streaming features are enabled only when the corresponding v2 native engine reports a real capability. The UI labels unavailable work explicitly; it does not expose functional-looking controls without a backend.

## Architecture

```text
apps/desktop                 Tauri 2 + Svelte 5 + TypeScript
apps/server                  Axum + Tokio API/signaling server
crates/sanser-*              Rust protocol/core/network/auth/storage modules
native/host-windows          Windows capture/encode/audio/input engine
native/client-macos          macOS receive/decode/render/audio/input engine
native/protocol              Portable C++ SNV2 authentication and STUN codecs
```

See [architecture](docs/sanser-2-architecture.md), [P2P v2 rollout](docs/p2p-v2.md), [SNV2](docs/sanser-2-protocol.md), [security](docs/sanser-2-security.md), and [performance](docs/sanser-2-performance.md).

## Prerequisites

- Rust stable 1.85 or newer
- Node.js 20.19 or newer and npm 11 (development tooling only)
- CMake 3.20 or newer
- macOS: Xcode command-line tools
- Windows: Visual Studio 2022 C++ desktop workload and Windows SDK

## Configuration

```bash
cp .env.example .env
chmod 600 .env
```

The API server requires a pooled PostgreSQL connection hosted by Neon:

```text
DATABASE_URL=postgresql://USER:PASSWORD@ep-EXAMPLE-pooler.REGION.aws.neon.tech/sanser?sslmode=require
```

SQLite and a bundled local database server are not used. The server rejects non-Neon hosts and database URLs without required TLS, and never prints the connection string.

`VITE_SANSER_SERVER_URL` is the public HTTPS address of the Sanser API, not the
Neon database address. Release builds bake this value into the app and remove
the endpoint editor, so users only install and sign in. `127.0.0.1:5174` is a
development default only. Neon hosts PostgreSQL; the Axum API must still be
deployed behind HTTPS on a server reachable by every client.

### Deploy once, install everywhere

1. Deploy `sanser-server` on an always-on HTTPS/WSS host and set
   `SERVER_HOST=0.0.0.0`, the Neon `DATABASE_URL`, strong application secrets,
   exact `ALLOWED_ORIGINS`, and enough bandwidth for relay traffic.
2. Verify `/api/v2/health` and `/api/v2/readiness` through the public domain.
3. Build the desktop with that domain in `VITE_SANSER_SERVER_URL`. For GitHub
   releases, set the repository variable `SANSER_API_URL` once.
4. Distribute the signed installer. Users do not need `.env`, Neon credentials,
   Node.js, Rust, or a local API server.

The GitHub macOS release also expects the signing secrets already referenced in
the workflow plus `APPLE_API_KEY`, `APPLE_API_ISSUER`, and
`APPLE_API_PRIVATE_KEY` (the `.p8` contents) for notarization.

## Development

Install frontend tooling:

```bash
npm install
```

Run the Rust API server:

```bash
npm run server:dev
```

Run the frontend alone or the complete Tauri application:

```bash
npm run dev
npm run desktop:dev
```

## Network modes

The desktop enables a route only after its packaged sidecar passes the native
capability probe. Authenticated Native Direct supports LAN, global IPv6 and
IPv4 Internet routes negotiated with STUN/UPnP or a fixed manual-forward port.
The native UDP-to-WSS bridge provides the production fallback without requiring
Tailscale, WebRTC or TURN.

The desktop exchanges bounded transient UDP candidates, runs STUN, bounded UPnP
IGD mapping and authenticated hole-punch checks on reserved IPv4/IPv6 sockets,
then hands the selected port to the packaged native engine. The Windows host
defaults to UDP `50000`, making a router rule stable across restarts.

- **Auto** tries Native Direct, then automatically opens the encrypted WSS relay if no UDP route succeeds.
- **Direct** uses only the reachable native route and never relays. If UPnP is unavailable, forward external UDP `50000` to UDP `50000` on the Windows PC (or use the port selected in **Settings → Host**).
- **Relay** skips direct probing and uses outbound WSS/443 from both devices.

For Internet-facing deployment, terminate HTTPS/WSS at a reverse proxy, use an
explicit `ALLOWED_ORIGINS`, disable proxy buffering for WebSocket upgrades and
run a single relay instance unless session affinity/shared relay state is
configured. Do not expose static production credentials to clients.

Keep `tauri://localhost` and `http://tauri.localhost` in `ALLOWED_ORIGINS` for
the packaged macOS and Windows webviews. Do not use `*`.

## Build and test

```bash
npm run check
npm run test
npm run build
npm run native:protocol:test
```

The npm commands keep Cargo output in the operating-system cache instead of
the repository, preventing cloud-synced workspaces from corrupting Rust
artifacts. An explicit `CARGO_TARGET_DIR` is still honored.

Build the current macOS client engine:

```bash
npm run native:client-macos:configure
npm run native:client-macos:build
```

On a Visual Studio Developer PowerShell, build the Windows host engine:

```powershell
npm run native:host-windows:configure
npm run native:host-windows:build
```

Bundle the desktop application with:

```bash
VITE_SANSER_SERVER_URL=https://api.your-domain.com npm run desktop:bundle
```

For a local unsigned verification bundle only:

```bash
npm run desktop:bundle:local
```

Expected release artifact names and unverified platform items are tracked in [the release ledger](docs/sanser-2-release.md). Local unsigned macOS builds are not notarized.

## Operating-system permissions

- Windows host requests capture, audio and firewall access only when hosting; administrator privileges are not required for normal operation.
- macOS client requests Accessibility/Input Monitoring when remote input capture is enabled and microphone permission only when microphone forwarding is enabled.
- Screen Recording is not requested on macOS until a future macOS host is actually enabled.

## Migration

The v2 migrator backs up legacy data, imports only validated non-secret preferences/device identity, and discards incompatible tokens and cryptographic material. Local preferences use an atomic JSON write; cloud metadata is written to Neon only after authentication. See [migration details](docs/sanser-2-migration.md).

## Troubleshooting

- Auto mode tries direct UDP first and uses the end-to-end encrypted WSS relay when NAT traversal fails. Redeploy the current server and install the matching desktop build on both devices before testing fallback.
- Direct mode fails across networks: enable UPnP/port forwarding and allow Sanser through the firewall, or switch back to Auto/Relay. Direct intentionally never uses the relay.
- Native capability is unavailable: build/install the platform v2 sidecar and inspect sanitized Diagnostics output.
- Database is not ready: verify the Neon pooled endpoint, TLS query, network allowlist and migration access without pasting credentials into logs/issues.

Sanser never sends realtime input, video or audio through PostgreSQL or the
signaling REST API. Relay mode forwards only bounded, end-to-end encrypted
frames through the in-memory WSS relay.
