# Sanser 2.2.25

Sanser is a low-latency remote desktop and remote game streaming platform. Version 2 is built as one Tauri application with a Svelte 5 interface, a Rust control plane, and native Windows/macOS media engines.

```text
Product:  Sanser 2.2.25
App ID:   com.sanser.desktop
Protocol: v2
Native:   Authenticated SNV2 media engines
```

## Capabilities

- Account, device and signaling data backed only by PostgreSQL on Neon.
- Local preferences stored as a bounded JSON file; secrets stay in Keychain/Credential Manager.
- Versioned `/api/v2` auth, devices, connection sessions, ICE and bounded WebSocket candidate signaling.
- Auto, Direct and Relay modes with native UDP, optional authenticated UDP relay, and an end-to-end encrypted WSS/443 fallback for blocked UDP. See [UDP relay setup](docs/udp-relay.md).
- Short-lived access tokens, refresh-token rotation/revocation and Argon2id passwords.
- Typed Tauri commands and sidecar allowlists; Node.js is not a packaged runtime.
- Bounded SNV2 packet, input, audio, retransmission and diagnostic primitives.
- Native Windows host/client and macOS host/client engines. Cross-platform H.264 sessions use the shared authenticated transport with separate OS backends; support is gated by packaged engine capabilities and server device roles.

Sanser 2.2.25 includes one-encoder screen sharing for up to three view-only
computers in the same account, common native diagnostics, bounded route reconnect,
authenticated input-only reset after ACK loss, viewer-close Disconnect and
reference-safe sharing dispatch. Relay recovery retains the real failure even
after native exits, and controlled server restarts send WebSocket Close 1012.
Version 2.2.16 adds Opus + PLC, continuously measured standby Direct routes,
bounded adaptive video FEC on Direct/UDP Relay, and group system audio with
independent viewer mute. Native Windows loopback capture and Mac SNV2 playback
complete the shared sound path. Real-device network and listening validation
remains necessary.
Version 2.2.17 adds opt-in group voice for the host and up to three viewers,
with independent Join/Unmute/Deafen/Leave controls. Microphones default off.
It also packages audio continuity/legacy audio, sharing recovery and sided Shift
fixes, plus optional Windows UAC support. See [group voice](docs/group-voice.md)
and [Windows UAC](docs/windows-uac.md) for consent, setup and remaining device checks.
Version 2.2.18 fixes missing legacy Mac audio, inverted Metal drawables, and
Join status races, and adds native floating voice/microphone controls on both
hosts and viewers.
Version 2.2.19 combines the host sharing indicator and voice controls into one
compact icon bar, while viewers only see voice controls. It also packages Mac
audio-output recovery, speaker/microphone echo cancellation, installer-managed
Windows administrator support and decoder warmup fixes.
Version 2.2.20 uses one group-voice playback clock for staggered speakers and
refines the compact sharing/voice controls. Closing a viewer ends its sharing
membership and frees the slot, while the host receives a named access alert
with a scoped Stop action. Desktop notifications can be enabled in Host.
Version 2.2.21 recovers transient Windows desktop/GPU capture failures, retains
rejected input releases for reset, and refreshes SNV2 pointer bounds after display
mode changes. These repairs do not establish the cause of every WAN latency spike.
Version 2.2.22 establishes UAC worker readiness before capture/input, separates
cold-start deadlines from steady responses, and shows the last helper failure
on the Mac capture-pause overlay. Legacy input no longer reports void fallback
APIs as proof of success; failed releases remain available for a later Reset.
Version 2.2.23 verifies the Windows privileged helper through SCM without opening
a SYSTEM process, and gives Windows floating voice/sharing controls the same
rounded compact presentation as Mac. The filtered-token CI fixture verifies an
actual elevated Cancel click even when the old SYSTEM process query is denied.
Version 2.2.25 adds recovery for hidden Mac audio converter/output-clock
failures and avoids unnecessary playback restarts on incomplete device metadata.
Both native engines distinguish temporary UDP loss from unusable sockets;
legacy video retains its partial frame for repair, and authenticated SNV2 tests
verify video recovery and reliable key transitions under injected socket faults.
The source baseline passed 39 Windows and 33 Mac native tests. Physical
three-direction soak and headphone listening tests are still required.
See [Release notes](docs/releases/v2.2.25.md),
[stability validation and remaining work](docs/session-stability-2.2.14.md)
and [implementation/validation progress](docs/multi-viewer-progress.md).
Installers and signed updater artifacts are produced by the release workflow.

Settings → Diagnostics separates control/input RTT, host input queue/work and
video stages without inventing missing measurements. A lost accepted native
session can retry with fresh authorization/candidates, at most twice; it reopens
the native window and retains Direct/Relay and view-only restrictions. See
[measurement semantics](docs/native-performance.md) and
[route recovery](docs/route-recovery.md).

SNV2 host input now has a separate bounded execution worker. Receipt ACK does
not certify OS execution; both stages are measured. Legacy and SNV2 share the
bounded retry formula. Fresh IDRs replace old queued video dependencies, stale
waiting chains expire and decoder cleanup exceptions are contained. See
[input engine](docs/input-engine.md) and
[video recovery and four-direction validation](docs/media-pipeline-validation.md).

Ordinary Mac controlling Windows retains Legacy; other ordinary desktop pairs
and group viewers use SNV2. OS backends remain native. Group sharing remains
view-only and now includes system sound with independent viewer mute. Physical capture/permissions, real two-computer latency,
NAT/VPN paths and group stability require retesting; 5–15 ms is not guaranteed.

On the host, select **Host → Go online → Start sharing → Invite**. Each invited
computer selects **Join**. Use **Remove** for one viewer or **Stop sharing** for
the group; a floating native Stop indicator remains visible on the host screen.
Update both desktops and the server: sharing requires `sharing-media` server
support and packaged `multiViewerHost` capability. Physical 2–3-machine and WAN
performance testing remains necessary.

Platform streaming features are enabled only when the corresponding v2 native engine reports a real capability. The UI labels unavailable work explicitly; it does not expose functional-looking controls without a backend.

## Architecture

```text
apps/desktop                 Tauri 2 + Svelte 5 + TypeScript
apps/server                  Axum + Tokio API/signaling server
crates/sanser-*              Rust protocol/core/network/auth/storage modules
native/host-windows          Windows capture/encode/audio/input engine
native/client-macos          macOS receive/decode/render/audio/input engine
native/desktop               Shared cross-platform session/transport engine
native/platforms/macos       macOS ScreenCaptureKit/VideoToolbox backend
native/platforms/windows     Windows Media Foundation/input/window backend
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

The GitHub release uses the updater signing secrets configured in the workflow.
The workflow supports Developer ID signing and optional notarization through
Apple repository secrets. Without those secrets, builds remain ad-hoc signed
and warn that screen-recording permissions may need to be granted again after
an update. See [Mac permissions and signing](docs/macos-permissions.md).

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

Current release scope and remaining platform validation are tracked in [the release notes](docs/releases/v2.2.17.md) and [sharing progress](docs/multi-viewer-progress.md). The [2.0.0 release ledger](docs/sanser-2-release.md) is historical. Local unsigned macOS builds are not notarized.

## Operating-system permissions

- Windows host requests capture, audio and firewall access only when hosting; administrator privileges are not required for normal operation.
- macOS client requests Accessibility/Input Monitoring when remote input capture is enabled and microphone permission only when microphone forwarding is enabled.
- macOS hosting is implemented with ScreenCaptureKit and VideoToolbox on macOS 12.3 or newer. Screen Recording is requested when hosting starts; Accessibility is needed when remote input is enabled.
- Windows includes a native client for viewing and controlling a remote desktop. Cross-platform sessions currently use H.264; system sound uses Opus/PCM, while HEVC support remains incomplete. Packaged engines and server role support must both be available. Automated builds and codec/transport tests do not replace real-device testing of each connection direction.
- Sanser 2.2.17 supports one shared capture/encoder and up to three independent view-only viewers. Sharing input is denied at the host; shared Opus audio is encoded once and each viewer can mute independently. Real GPU/capture/network testing remains necessary.

## Migration

The v2 migrator backs up legacy data, imports only validated non-secret preferences/device identity, and discards incompatible tokens and cryptographic material. Local preferences use an atomic JSON write; cloud metadata is written to Neon only after authentication. See [migration details](docs/sanser-2-migration.md).

## Troubleshooting

- Auto mode tries direct UDP first and uses the authenticated UDP relay, or end-to-end encrypted WSS fallback, when NAT traversal fails. Redeploy the current server and install the matching desktop build on both devices before testing fallback.
- Direct mode fails across networks: enable UPnP/port forwarding and allow Sanser through the firewall, or switch back to Auto/Relay. Direct intentionally never uses the relay.
- Native capability is unavailable: build/install the platform v2 sidecar and inspect sanitized Diagnostics output.
- Database is not ready: verify the Neon pooled endpoint, TLS query, network allowlist and migration access without pasting credentials into logs/issues.

Sanser never sends realtime input, video or audio through PostgreSQL or the
signaling REST API. Relay mode forwards only bounded, end-to-end encrypted
datagrams through the configured UDP relay or encrypted frames through the
in-memory WSS relay.
