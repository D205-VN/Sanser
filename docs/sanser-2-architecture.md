# Sanser 2.0.8 architecture

Sanser 2 is one product with a thin Tauri shell, typed Svelte UI, Rust control plane, and platform-native media engines. Node.js is a development tool only and is not part of a packaged runtime.

## Workspace boundaries

| Area | Responsibility | Runtime |
| --- | --- | --- |
| `apps/desktop` | Tauri lifecycle, secure commands, Svelte UI | Rust + WebView |
| `apps/server` | API v2, auth, presence, sessions, ICE/signaling | Rust/Axum/Tokio |
| `crates/sanser-*` | Shared protocol, P2P policy, config, auth, storage, queues, diagnostics | Rust |
| `native/host-windows` | Capture, encode, WASAPI, Windows input injection | C++20 |
| `native/client-macos` | Receive, VideoToolbox, Metal, CoreAudio, native input | Objective-C++20 |
| `native/protocol` | Portable C++ SNV2 and STUN wire codecs | C++20 |

The frontend never receives native video frames and is not the primary realtime input transport. Tauri commands accept typed, validated data and can launch only known sidecars with whitelisted arguments.

## Control and media planes

The control plane owns authentication, device presence, consent, session negotiation, candidate exchange, relay authorization, and state notifications. PostgreSQL never carries realtime input, video, or audio.

The currently shipped media plane selects one route:

1. Authenticated Native Direct over LAN, global IPv6, STUN/UPnP or a fixed
   manual-forward UDP route.
2. An authenticated WSS/443 packet relay for CGNAT, blocked UDP and other hard
   networks; native WebRTC/libdatachannel remains an optional future transport.
3. A clear failure when neither route is available.

The P2P v2 coordinator validates and scores transient candidates, performs
authenticated hole-punch checks on reserved sockets, and hands the selected
endpoint to the native engine. Auto falls back to a local UDP-to-WSS bridge
when those checks fail. See `docs/p2p-v2.md`.

Input uses separate bounded lanes: reliable ordered keys/buttons, latest-state-wins mouse/gamepad, audio, video control, video payload, and diagnostics. Media backpressure can drop expired video but cannot block reliable input.

## Storage

- PostgreSQL hosted by Neon is the only server database for account/device/session data.
- The desktop stores bounded non-secret preferences in an atomic local JSON file; this is not an application database.
- Secrets belong in Keychain/Credential Manager or process memory. Databases contain token digests, never raw access/refresh tokens.

## Current verification status

This document describes the target and implemented workspace boundaries. A feature is release-ready only when its implementation and tests exist. Platform media claims require Windows/macOS hardware testing and are tracked in `docs/sanser-2-release.md`.
