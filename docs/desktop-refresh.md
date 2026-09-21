# Desktop workspace refresh

This change updates the shared desktop layout, device management, account forms,
settings feedback, session presentation, diagnostics, and updater.

## Behavior changes

- Device registration handles older servers that explicitly reject `deviceRole`
  or `crossPlatform`: Mac clients and Windows hosts retry once with the legacy
  payload and the same device ID. Other roles require a server upgrade and report
  that requirement instead of registering a misleading role. Authentication,
  network, and unrelated validation failures do not trigger this fallback.
- The workspace opens directly to the computer list, search, filters, and
  connection actions. Marketing banners, decorative diagrams, duplicate counters,
  runtime labels, and codec summaries have been removed from everyday screens.
  Device cards retain names, operating systems, availability, last-seen dates,
  pinning, management, and connection actions. Network and quality defaults remain
  in Settings; failed registration still exposes a retry action.
- Sign-in is a compact form. Host shows sharing controls and access requests;
  router setup is under advanced Host settings. Controls for unimplemented
  language, microphone, gamepad, capture selection, and lock-on-disconnect features
  have been removed. Audio controls appear only when the runtime reports support.
  Diagnostics retains technical troubleshooting information, but its empty
  placeholder timing rows have been removed. About retains product and version
  information. Unused decorative CSS has been removed as well.
- Diagnostics is available in Settings rather than the main navigation. Export
  and collection controls remain immediately available there; technical status
  and events expand on demand. Device and session troubleshooting links open
  Settings with connection details expanded, without unmounting session monitoring.
- Navigation resets the workspace scroll position without unmounting session
  monitoring. Network and quality changes still apply to new sessions.
- The session view shows approval, route negotiation, and remote-window progress.
  Opening the native process is labelled "Window open"; it does not claim that
  media has arrived. Negotiation can be cancelled, closes signaling, and releases
  its reserved P2P attempt. Late authorization cannot launch an engine, and new
  connection requests remain blocked until the previous preparation finishes.
  Session statistics appear only when real metrics exist; the unrelated launcher
  fullscreen button and empty-session toolbar have been removed.
- Session monitoring remains mounted when users navigate away from Active session.
  A pending or running connection blocks another connection request and provides a
  return-to-session action.
- Requests completing after sign-out cannot launch a new client engine or restore
  a cleared connection. Host startup checks its lifecycle before publishing success.
- Host route negotiation is cancelled on shutdown, Stop session, and remote
  disconnection. Late credentials cannot start a relay or engine. Registration
  cleanup and pending native launches finish before a new host lifecycle starts;
  stale polling and heartbeat results cannot overwrite a newer lifecycle. Native
  startup errors remain visible across successful polls. Failed native shutdown
  retains the sharing state and reports an error so users can retry stopping.
  A server role mismatch shows a sharing-unavailable state and Check again action.
- Authenticated API requests retry once after a shared refresh-token rotation.
  Temporary startup or refresh-service outages preserve stored credentials; rejected
  refresh credentials require sign-in again. Credential writes are serialized with
  deletion, preventing a pending write from winning over sign-out.
- Sign-in now offers an unchecked-by-default **Keep me signed in** checkbox.
  Unchecked sessions stay in memory; checked sessions use native credential
  storage and expire after seven days without authenticated activity. macOS and
  Windows keyring backends are explicitly enabled. See
  [remembered-sign-in.md](remembered-sign-in.md) for behavior and verification.
- Preference writes are serialized. Failed writes restore the last persisted state
  and report an error instead of silently displaying an unsaved value.
- Computer lists read all cursor pages, distinguish empty accounts from empty
  filters, show counts and pinned computers, and keep stale list responses from
  overwriting rename/pin/remove actions. Repeated device IDs across cursor pages
  are deduplicated. The current host/client is excluded.
- Competitive uses 720p / 120 FPS / 12 Mb/s, Balanced uses 1080p / 60 FPS / 20 Mb/s,
  and Quality uses 1440p / 60 FPS / 40 Mb/s on both endpoints. Auto uses the Balanced
  defaults. Codec selection is unchanged. Editing video values selects Custom;
  the host controls custom capture settings because the session API does not carry
  the client's full custom video settings. Both desktops need this frontend change
  to use matching presets.
- Registration confirms the password and supports show/hide controls. Changing a
  development server clears the previous sign-in before accessing the new server.
- Diagnostics can refresh process status instead of displaying only the startup
  snapshot. Input settings direct users to the remote window's release shortcut
  instead of displaying a nonfunctional custom-shortcut control.
- The updater waits until active sessions finish, uses a keyboard-accessible dialog,
  handles downloads without Content-Length, and offers an explicit restart action.

## Verification

Automated checks:

```sh
npm run desktop:check
npm run desktop:lint
npm run desktop:test
npm run desktop:build
git diff --check
```

Regression tests cover outage recovery, token rotation and concurrent renewal,
refresh completion after logout, failed settings persistence, stale session
responses, stream presets, password visibility/confirmation, search-filter reset,
session monitoring while navigating the workspace pages, switching card/list
views, cancellation during candidate gathering/exchange, and authorization
completion after cancellation, and registration compatibility with older servers.
The latest frontend run passed all 86 tests,
type checking, lint, and the production frontend build.

The tests use mocked APIs and native commands. Real Windows-to-macOS video, audio,
input, NAT traversal, signed updater installation, and visual layouts in the Tauri
webview still require device verification. Browser automation was unavailable in
this editing session; component interaction tests do not establish pixel-level
layout correctness. A macOS preview with embedded frontend assets was built and
opened; its sign-in window was captured and visually checked. The authenticated
workspace has component-test coverage but was not visually checked in that run.
No server deployment or installer publication is part of this change. Local macOS sidecars are rebuilt for preview; see
[cross-platform-native.md](cross-platform-native.md) for the new transport and its
verification limits.
