# Sanser 2.0.0 architecture

Sanser 2 is one product with a thin Tauri shell, typed Svelte UI, Rust control plane, and platform-native media engines. Node.js is a development tool only and is not part of a packaged runtime.

## Workspace boundaries

| Area | Responsibility | Runtime |
| --- | --- | --- |
| `apps/desktop` | Tauri lifecycle, secure commands, Svelte UI | Rust + WebView |
| `apps/server` | API v2, auth, presence, sessions, ICE/signaling | Rust/Axum/Tokio |
| `crates/sanser-*` | Shared protocol, config, auth, storage, queues, diagnostics | Rust |
| `native/host-windows` | Capture, encode, WASAPI, Windows input injection | C++20 |
| `native/client-macos` | Receive, VideoToolbox, Metal, CoreAudio, native input | Objective-C++20 |
| `native/protocol` | C++ SNV2 wire codec shared by native engines | C++20 |

The frontend never receives native video frames and is not the primary realtime input transport. Tauri commands accept typed, validated data and can launch only known sidecars with whitelisted arguments.

## Control and media planes

The control plane owns authentication, device presence, consent, session negotiation, SDP/ICE exchange, TURN credentials, and state notifications. PostgreSQL/SQLite never carry realtime input, video, or audio.

The media plane selects one route:

1. SNV2 for an authenticated direct route.
2. Native WebRTC/libdatachannel for ICE direct or TURN relay.
3. A clear failure when neither route is available.

Input uses separate bounded lanes: reliable ordered keys/buttons, latest-state-wins mouse/gamepad, audio, video control, video payload, and diagnostics. Media backpressure can drop expired video but cannot block reliable input.

## Storage modes

- Local mode uses SQLite and does not require an account or PostgreSQL.
- Shared mode uses PostgreSQL for account/device/session data.
- Secrets belong in Keychain/Credential Manager or process memory. Databases contain token digests, never raw access/refresh tokens.

## Current verification status

This document describes the target and implemented workspace boundaries. A feature is release-ready only when its implementation and tests exist. Platform media claims require Windows/macOS hardware testing and are tracked in `docs/sanser-2-release.md`.
