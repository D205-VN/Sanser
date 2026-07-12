# Sanser 2.0.2 architecture

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

The control plane owns authentication, device presence, consent, session negotiation, SDP/ICE exchange, TURN credentials, and state notifications. PostgreSQL never carries realtime input, video, or audio.

The currently shipped media plane selects one route:

1. The authenticated Native Direct compatibility wire for a reachable IPv4 route.
2. Native WebRTC/libdatachannel for ICE direct or TURN relay.
3. A clear failure when neither route is available.

The opt-in P2P v2 target replaces that compatibility wire with one SNV2 UDP
component. Candidate validation, scoring, state transitions and transient
signaling are implemented; native socket gathering, hole punching and SNV2
multiplexing remain capability-gated. See `docs/p2p-v2.md`.

Input uses separate bounded lanes: reliable ordered keys/buttons, latest-state-wins mouse/gamepad, audio, video control, video payload, and diagnostics. Media backpressure can drop expired video but cannot block reliable input.

## Storage

- PostgreSQL hosted by Neon is the only server database for account/device/session data.
- The desktop stores bounded non-secret preferences in an atomic local JSON file; this is not an application database.
- Secrets belong in Keychain/Credential Manager or process memory. Databases contain token digests, never raw access/refresh tokens.

## Current verification status

This document describes the target and implemented workspace boundaries. A feature is release-ready only when its implementation and tests exist. Platform media claims require Windows/macOS hardware testing and are tracked in `docs/sanser-2-release.md`.
