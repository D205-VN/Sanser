# Bidirectional desktop engines

The desktop now packages a host and client engine on each supported platform.
The server registers separate host/client identities and returns `wireProtocol`
with native session credentials. Old Windows-host/macOS-client sessions retain
the legacy implementation. Other pairs require both engines to advertise
`crossPlatform` and use the authenticated SNV2 transport in `native/desktop`.
OS-specific implementations live in `native/platforms/macos` and
`native/platforms/windows`; see [native-platforms.md](native-platforms.md) for
the separate build targets, rendering changes, and verification limits.

| Controlling client | Remote host | Selected implementation |
| --- | --- | --- |
| macOS | Windows | Existing native path, H.264/HEVC |
| Windows | Windows | New SNV2 H.264 path |
| macOS | macOS | New SNV2 H.264 path |
| Windows | macOS | New SNV2 H.264 path |

This table describes implemented negotiation and code paths, not a certification
of successful streaming on four physical device pairs. Windows compilation and
physical device verification remain outstanding in this macOS workspace.

## What changes

- macOS host uses ScreenCaptureKit (macOS 12.3+) and VideoToolbox H.264 encoding.
  Screen Recording permission is requested when capture starts; Accessibility is
  needed only when remote input is enabled. Capability probes request no permission.
- Windows client decodes H.264 with Media Foundation and renders the remote display
  in a native window. Existing Windows capture/encoding is reused for SNV2 hosts.
- Peers use directional AES-256-CBC encryption with HMAC-SHA256 authentication,
  bounded frame assembly, replay windows, reliable ordered keyboard/button events,
  keyframe recovery and keepalive timeouts. Credentials stay in process memory/env,
  never in command-line arguments. The existing encrypted WSS bridge is reused.
- Input uses normalized screen coordinates and USB HID key codes. Focus loss and
  disconnection release pressed keys/buttons. Control+Option+Escape on macOS or
  Ctrl+Alt+Escape on Windows releases input; click the stream to resume.
- The shell gracefully stops new engines through a private stdin pipe before a
  bounded forced shutdown. Startup failures expose permission/codec errors.
- Device listings block client-only targets and incompatible codecs before starting
  a session. Registration preserves user-renamed device names.
- Cross-platform sessions currently use H.264 and absolute keyboard/mouse only.
  Relative mouse, audio, clipboard and gamepad are not implemented in the new path.
  Audio settings are disabled because the current single-socket launcher does not
  connect the legacy engines' separate audio ports either.
- Direct route selection uses IPv4 because both media backends currently bind IPv4
  sockets; a successful IPv6 probe no longer causes a later engine launch failure.
- Custom video settings still come from the host. Preset profiles match on both
  endpoints. One primary display is captured; monitor/window selection is not added.

## Upgrade and verification

Deploy the updated server with migration `0004_device_roles.sql` before enabling
new connection directions. Install matching updated desktop builds on both ends.
The desktop rejects unsupported negotiation from an older server. No production
server/database was changed and no installer was published by this work.

Checks performed locally:

- Frontend type checking, lint, production build and 40 component/unit tests.
- Rust workspace clippy and tests, including all four platform negotiations,
  role rejection, credential handling, relay and native launch argument checks.
- Native macOS client and host compile and advertise the expected capabilities.
- Native protocol tests verify encryption round trips, tampering, reflected packets,
  wrong credentials, replay rejection, input validation and bounded frame assembly.
- UDP loopback test starts two authenticated peers, transfers fragmented video and
  ordered key events, then verifies input reset on disconnect. No real screen capture
  or OS input injection occurs in this test.
- A synthetic macOS video test uses the actual VideoToolbox encoder and decoder,
  transfers H.264 through encrypted loopback peers and checks decoded dimensions
  and pixel colors. This does not exercise ScreenCaptureKit permission or capture.

The Neon integration suite is explicitly ignored in the default test run. Configure
`TEST_DATABASE_URL` for a dedicated test database and run
`node scripts/run-cargo.mjs test -p sanser-server --test api -- --ignored` to exercise
it. This explicit run fails if the test URL is missing; a normal Cargo run does
not verify migration or database-backed API behavior.
Physical Windows/macOS streaming, NAT/router transitions, permissions, signed
installation/update and pixel-level UI appearance still need device verification.
CI includes both native binaries and protocol/loopback tests on Windows and macOS.

Local commands:

```sh
npm run desktop:stage-sidecar
node scripts/run-cargo.mjs clippy --workspace --all-targets --all-features -- -D warnings
node scripts/run-cargo.mjs test --workspace --all-features
npm run desktop:check && npm run desktop:lint && npm run desktop:test
npm run desktop:build
cmake -S native/desktop -B /tmp/sanser-desktop-native-build -DBUILD_TESTING=ON
cmake --build /tmp/sanser-desktop-native-build --parallel
ctest --test-dir /tmp/sanser-desktop-native-build --output-on-failure
```
