# Sanser 2.0.0

Sanser is a low-latency remote desktop and remote game streaming platform. Version 2 is built as one Tauri application with a Svelte 5 interface, a Rust control plane, and native Windows/macOS media engines.

```text
Product:  Sanser 2.0.0
App ID:   com.sanser.desktop
Protocol: v2
Native:   SNV2
```

## Capabilities

- Local/LAN mode backed by SQLite, with no PostgreSQL installation required.
- Shared/cloud mode backed by PostgreSQL.
- Versioned `/api/v2` auth, devices, connection sessions, ICE and WebSocket signaling.
- Auto, Direct and Relay network modes with STUN/TURN.
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
native/protocol              Portable C++ SNV2 codec/authentication
```

See [architecture](docs/sanser-2-architecture.md), [SNV2](docs/sanser-2-protocol.md), [security](docs/sanser-2-security.md), and [performance](docs/sanser-2-performance.md).

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

For a local installation, keep:

```text
STORAGE_MODE=local
SQLITE_PATH=./data/sanser.db
NETWORK_MODE=auto
```

Shared mode additionally requires a PostgreSQL connection:

```text
STORAGE_MODE=shared
DATABASE_URL=postgresql://USER:PASSWORD@HOST:5432/sanser
```

The server validates version, protocol, addresses, URLs, TTLs, origins, storage and relay configuration at startup without printing secret values.

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

- **Auto** tries signed LAN discovery, direct/private routes, ICE direct, STUN candidates, then TURN UDP/TCP/TLS. SNV2 is selected only for an authenticated direct route; otherwise the session falls back to WebRTC.
- **Direct** disables TURN and permits LAN, public routes and STUN. NAT/firewall failure is reported clearly.
- **Relay** requires TURN, prefers UDP and falls back to TCP/TLS. It uses WebRTC because SNV2 does not relay media through the signaling server.

For Internet-facing shared mode, terminate HTTPS/WSS at a reverse proxy, use an explicit `ALLOWED_ORIGINS`, and configure `TURN_SHARED_SECRET` so the server mints short-lived TURN credentials. Do not expose static production credentials to clients.

## Build and test

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
npm run desktop:check
npm run desktop:lint
npm run desktop:test
npm run desktop:build
npm run native:protocol:test
```

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
npm run desktop:bundle
```

Expected release artifact names and unverified platform items are tracked in [the release ledger](docs/sanser-2-release.md). Local unsigned macOS builds are not notarized.

## Operating-system permissions

- Windows host requests capture, audio and firewall access only when hosting; administrator privileges are not required for normal operation.
- macOS client requests Accessibility/Input Monitoring when remote input capture is enabled and microphone permission only when microphone forwarding is enabled.
- Screen Recording is not requested on macOS until a future macOS host is actually enabled.

## Migration

The v2 migrator backs up legacy data, imports only validated non-secret preferences/device identity, discards incompatible tokens and cryptographic material, and writes migration version 2 transactionally. See [migration details](docs/sanser-2-migration.md).

## Troubleshooting

- `relay requires TURN`: configure `TURN_URLS` plus a shared secret or development credential pair.
- Direct connection fails across networks: verify firewall/NAT, then use Auto or Relay.
- Native capability is unavailable: build/install the platform v2 sidecar and inspect sanitized Diagnostics output.
- Database is not ready: verify `STORAGE_MODE`, path permissions, migration access and the PostgreSQL URL without pasting credentials into logs/issues.

Sanser never sends realtime input, video or audio through PostgreSQL or the signaling REST API.
