# GitHub UI and platform feature survey — 2026-11-03

Research for Sanser 2.3.3. This is a recommendation record, not an implementation
or a claim that these projects solve Sanser's latency/audio bugs.

Selection uses public repository popularity, recent repository activity and fit
with Sanser's Svelte 5/Tauri desktop and Flutter mobile clients. Star counts below
are a snapshot, not daily Trending ranks. A push date does not prove maintenance
quality or release readiness.

## UI candidates

| Project | Stars | Observed last push (UTC) | Proposed Sanser use |
| --- | ---: | --- | --- |
| [shadcn-svelte](https://github.com/huntabyte/shadcn-svelte) | 9,236 | 2026-11-02 | Desktop device cards, settings, menus and dialogs; select components rather than replacing the application |
| [Bits UI](https://github.com/huntabyte/bits-ui) | 3,644 | 2026-11-03 | Accessible unstyled Svelte primitives if preserving Sanser's CSS; underlying alternative to taking full styled components |
| [Forui](https://github.com/duobaseio/forui) | 2,411 | 2026-11-03 | Primary mobile UI candidate: compact sheets, buttons, settings and shared theme tokens |
| [Flutter ShadcnUI](https://github.com/nank1ro/flutter-shadcn-ui) | 2,837 | 2026-10-30 | Alternative mobile component set; compare one representative screen against Forui before choosing one |
| [Lucide](https://github.com/lucide-icons/lucide) | 25,481 | 2026-11-03 | Consistent icon designs for mic, speaker, display, transport, keyboard and disconnect |

Forui's README requires Flutter 3.47.0+ for 0.26.0+. Sanser's CI pins Flutter
3.47.7; dependency resolution and an actual device prototype remain necessary.
Desktop and Flutter components share visual tokens and icon assets, not widget
source. Native C++/AppKit/Win32 floating panels need a separate implementation of
the same visual specification. Keep video, decode and audio work out of UI code.

Licenses observed: shadcn-svelte/Bits UI/Flutter ShadcnUI code MIT; Forui code MIT
with separate font/icon notices in its LICENSE; Lucide ISC plus notices for
Feather-derived icons. Check the exact pinned version and retain its notices.

## Functional references

| Project | Stars | What to inspect | Integration consequence |
| --- | ---: | --- | --- |
| [Microsoft PowerToys](https://github.com/microsoft/PowerToys) | 140,464 | Command Palette, Keyboard Manager, FancyZones, Awake and Shortcut Guide | Reference for a Windows session action panel and shortcut profiles; not a ready-made Sanser plugin |
| [Hammerspoon](https://github.com/Hammerspoon/hammerspoon) | 16,375 | macOS automation bridge and system extensions | Reference for Mac hotkeys, application/window actions and permission handling; native macOS implementation required |
| [RustDesk](https://github.com/rustdesk/rustdesk) | 127,968 | Cross-platform client UX, clipboard, file transfer and platform boundaries | Inspect algorithms/behavior; not a drop-in SNV2 transport |
| [Sunshine](https://github.com/LizardByte/Sunshine) | 42,941 | Hardware encoding, host configuration and pairing | Reference for capture/encode and host lifecycle |
| [Moonlight PC](https://github.com/moonlight-stream/moonlight-qt) | 19,366 | Decode/render, pointer capture, host shortcuts, controller and multitouch support | Reference for native streaming and input, not a Flutter UI library |
| [Moonlight Android](https://github.com/moonlight-stream/moonlight-android) | 7,416 | Android streaming client and device-specific playback/input behavior | Compare against Sanser on the same phone/network/media settings |
| [Moonlight iOS](https://github.com/moonlight-stream/moonlight-ios) | 1,705 | Native iOS streaming and device integration | Same-device benchmark/reference; preserve Sanser's Flutter client shell |
| [LiveKit server](https://github.com/livekit/livekit) / [Flutter SDK](https://github.com/livekit/client-sdk-flutter) | 22,009 / 422 | Multi-user voice/audio lifecycle and Flutter integration | Requires LiveKit Cloud or a self-hosted server plus token/session integration; prototype group voice independently of desktop video |

PowerToys/Hammerspoon code licenses are MIT. LiveKit server/Flutter SDK licenses
are Apache-2.0. RustDesk is AGPL-3.0; Sunshine/Moonlight are GPL-3.0. Treat
architecture research separately from incorporating third-party code, and review
the exact files/dependencies before any reuse.

Primary source examples:

- [RustDesk clipboard design](https://github.com/rustdesk/rustdesk/blob/master/libs/clipboard/README.md).
- [PowerToys utilities](https://github.com/microsoft/PowerToys/blob/main/README.md).
- [Hammerspoon system bridge](https://github.com/Hammerspoon/hammerspoon/blob/master/README.md).
- [Moonlight PC feature list](https://github.com/moonlight-stream/moonlight-qt/blob/master/README.md).
- [LiveKit Flutter platforms and server requirement](https://github.com/livekit/client-sdk-flutter/blob/main/README.md).
- [Forui requirements and licenses](https://github.com/duobaseio/forui/blob/main/README.md).

## Proposed delivery order

1. Define shared typography, spacing, colors, icons and connection states. Prototype
   Computers, Active Session and mobile Settings; measure overlay/UI costs.
2. Build a capability-aware session action panel: supported shortcuts, audio route,
   display selection and clear reconnect/disconnect feedback. Native desktop
   floating controls follow the same design but retain native implementation.
3. Complete useful everyday features: authenticated text clipboard, consented file
   transfer, shortcut ownership/release and full Unicode/IME input. Transfer traffic
   needs its own bounded low-priority path; it must not delay input/audio/video.
4. Deepen platform behavior: Mac Command/Option/window actions; Windows Win/Alt,
   elevated-app input/UAC service lifecycle; mobile external keyboard/controller,
   haptics, tablet layout and network-change recovery. Validate each capability
   against native OS permissions and actual devices.
5. Prototype group-voice improvements and measure performance: same-network A/B
   of existing audio versus a LiveKit voice prototype, identical hardware/network,
   P50/P95 and loss/underrun/drop counters. Choose based on evidence, not stars.

Current mobile already has touchpad, zoom, extended keyboard keys and a
keyboard-mapped game overlay. These are not new roadmap items. It still advertises
`gamepad: false`, and its keyboard explicitly supports ASCII rather than full IME.
This survey does not change or release application code.

## Implementation follow-up — 2026-11-05

The observations above remain the original research snapshot. Subsequent source
work is tracked here and in [design/README](../../design/README.md); no release
is implied by this table. Follow-ups are chronological; later status supersedes
earlier remaining-work statements.

| Phase | Source status | Outstanding validation/scope |
| --- | --- | --- |
| 1 — shared foundation | Shared CSS/Dart/native tokens and controls | Device/OS visual checks |
| 2 — session actions | Capability-aware audio, input, files and diagnostics | OS audio route and remote monitor selection need native capabilities |
| 3 — everyday features | Committed Unicode, manual consented clipboard/files, mobile local IME, Mac text mode, Windows draft editor | Full host-context IME remains separate; Windows runtime and device/provider checks pending |
| 4 — platform behavior | Mobile keyboard/haptics/tablet/recovery, controller keyboard profile, host shortcuts, SNV2 analog core, optional Windows backend and negotiated mobile/native-desktop analog capture/UI | macOS virtual controller host, rumble and real device/Windows runtime validation |
| 5 — voice/performance | Native counters/offline A/B (5a), isolated LiveKit browser voice prototype (5b) | Real server/device trials, native membership/loopback integration and cross-provider A/B evidence |

The Windows draft has a standalone CMake/CI test job independent of capture and
audio SDKs. Its executable cross-builds on Mac; Windows execution remains
pending. Mobile keyboard tests simulate framework events; they do not prove
USB/Bluetooth delivery or streaming latency on a physical phone.

Phase 4b adds [local touch feedback](../mobile-touch-feedback.md), off by default
and persisted in Settings. Mock-channel tests cover a stalled feedback reply
without blocking input; no physical haptics or latency measurement is implied.

Phase 4c adds [adaptive tablet layout](../mobile-tablet-layout.md): navigation
rail, readable device columns, bounded Settings and larger floating controls.
Tests cover resize/split-screen/keyboard/large-text behavior and preserve live
input ownership; real tablet/native-video validation remains.

Phase 4d adds [network-change recovery](../mobile-network-recovery.md) for an
already streaming accepted session. Fresh readiness/candidates, two attempts,
45-second deadline and input/event generations prevent stale work from entering
the reopened media engine. OS handoff/device tests and seamless migration remain.

Phase 4e1 adds [physical controller → keyboard](../mobile-controller-keys.md):
native iOS/Android capture, neutral arming, hysteresis, independent key ownership
and scoped capture generations. It reuses HID input and keeps `gamepad: false`;
virtual Xbox/analog/rumble and host controller backends are **not** implemented.

Phase 4f adds [mobile → host shortcuts](../mobile-host-shortcuts.md): explicit
Windows/macOS window/editing actions, matched host metadata, key ownership and
reset-epoch cancellation. It reuses existing HID input; this does not add native
desktop window management or complete phase 4. Phase 5 voice/performance A/B
remains separate; these source checks are not measured latency improvements.

Phase 4g adds [SNV2 native desktop host shortcuts](../native-session-shortcuts.md):
modeless AppKit/Win32 panels and whole-group reliable input admission. A reset
on entry releases physical input; balanced actions do not reset/erase preceding
queued actions. Mac UI/UDP fault regression tests pass; Windows cross-builds
with runtime/MSVC validation pending. Legacy support follows separately below.

Phase 4h adds [Legacy Mac → Windows shortcuts](../legacy-session-shortcuts.md):
the same AppKit panel, Windows-only fixed named operations, authenticated host
capability and predecessor ACK barriers. Rejected/expired operations cancel
dependent work; physical capture resumes only on a stream click. Native Mac
tests/build pass, with Windows host syntax/UI cross-build checks; Windows runtime
and real two-machine validation remain. No installed sidecar or release changes.

Phase 4i1 adds the [SNV2 analog core and optional Windows host backend](../native-gamepad-core.md):
authenticated capability, a separate latest-state slot, reset-generation fences,
neutral arming and a 250 ms idle watchdog. Loopback/fault tests and portable
backend tests pass; Mac builds and Windows cross-build/syntax checks are available.
Mobile still uses keyboard mapping; analog capture/UI, rumble and real Windows
driver testing are not complete. Archived ViGEm compatibility is optional and
does not install a driver or become a new required dependency.

Phase 4i2 connects [mobile analog capture and UI](../mobile-analog-controller.md)
through Swift/C and Kotlin/JNI. The native latest-state sender refreshes held
state every 40 ms; Flutter receives status rather than axes. Host capability
gates the action, capture IDs/generations isolate old work and capability loss
disarms without auto-restart. Flutter/native tests, unsigned iOS build and Kotlin
compile pass. Real device/driver, native desktop analog and rumble work remains.

Phase 4i3 adds [native desktop analog capture/UI](../native-desktop-controller.md)
on the SNV2 Mac and Windows viewers. The shared mobile/native sender keeps
neutral arming, latest-state delivery and held heartbeats; desktop adds bounded
Reset admission, focus/generation gates and a 200 ms stale-poll cutoff. Nine
focused native desktop suites and two native mobile regressions pass. Mac builds,
Windows UI/model tests cross-build and the Windows backend syntax-checks; real
controller/driver/MSVC/runtime and latency validation remain. Legacy routing is
unchanged. Phase 5 voice/performance A/B has not started.

Phase 5a adds [voice/performance evidence](../voice-performance-ab.md): live native
voice/system-sound counter deltas, configuration epochs, a separate interval
worker-gap measurement and bounded exported summaries. Offline A/B comparison
checks requested media/route conditions and observation coverage before emitting
numeric changes; missing evidence is not zero or a passing quality score. Rust,
UI/report, offline analysis and synthetic native voice tests pass. LiveKit
prototype/adaptation and actual acoustic/device A/B remain for phase 5b; no
end-to-end latency or hardware audio fix is claimed by this increment.

Phase 5b adds the isolated [Sanser Voice Lab](../../apps/voice-lab/README.md),
using pinned LiveKit JS 2.23.0 with its own lockfile and no desktop/mobile runtime
dependency. Listen-only Join, explicit mic consent, cancellation/reconnect/hidden
page cleanup and microphone-only subscriptions have fake-device regression tests.
The responsive interface builds and has desktop/narrow Chromium previews. A
bounded WebRTC stats export keeps SFU RTT, receiver jitter and jitter-buffer
residence separate; native and browser audio counters are not falsely unified.
No actual LiveKit room/endpoint, physical audio or same-device quality/latency
A/B was exercised. Production Sanser auth, floating controls, browser-process
loopback exclusion and cross-provider measurement integration remain.

Phase 5b1 adds experimental [Sanser-account admission](../../apps/voice-lab/README.md#join-through-a-sanser-account-phase-5b1):
an off-by-default server credential endpoint, owned/fresh host or accepted live
sharing viewer checks, 60-second microphone-only grants and opaque room/identity
mapping. The lab logs in separately, selects an existing sharing role, requests
credentials without manual JWT paste and cancels stale work/voice on Leave or
cooperative membership rejection. Server unit tests and fake HTTP/DOM tests
pass; the database authorization test compiles but awaits a dedicated test DB.
This supersedes the earlier absence of any account/token integration, **not**
production readiness: SFU-enforced revocation (including cached/refreshed tokens),
real server/audio A/B, native floating/loopback integration and mobile remain.
Polling and short JWT TTL do not revoke a connected LiveKit participant.

Phase 5b1 validation follow-up (2026-11-06): all ten database API tests now pass
on an isolated local PostgreSQL 17.11 cluster. The real-schema test exposed an
integer/boolean mismatch in the new online checks, which is fixed. Existing
auth/sharing/session/cleanup integration cases also pass. Voice Lab now has a
separate CI check/test/build job; no SFU/media/hardware evidence is implied.
At the user's request, the next release is **held until Voice Lab is completed
and tested**, rather than shipping only the completed app features with the lab
disabled. Product version and current public release remain 2.3.3.

Phase 5b2 adds a [voice room revision fence](../../apps/voice-lab/README.md#room-revision-fence-phase-5b2).
Database triggers change the next room after accepted-member removal, deletion,
session/identity reassignment or group stop. Grant issuance locks the group before
rechecking membership, covering admission racing with removal. Tokens for the old
room do not authorize the new room; the lab detects a changed revision, stops mic
and asks for explicit rejoin. New viewers and unchanged revisions do not restart
the round. All ten real PostgreSQL API tests (including the race) and 35 fake
SDK/HTTP/DOM/stats tests pass; build/strict Clippy pass. This does **not** close
an old SFU room or terminate its connected participants. Live SFU administration,
logout/account revocation, native integration and real multi-person audio tests
remain release gates; the release hold stays in effect.

Phase 5b3 (2026-11-06) implements durable SFU room closure: issued rooms are
registered under the admission lock, revision/group-deletion triggers retire
them, and a leased PostgreSQL outbox retries LiveKit `DeleteRoom` after failures
or API restarts. Network calls hold no DB locks and admin credentials never
reach clients. Strict Clippy and 36 server unit tests pass. All 11 PostgreSQL API
tests pass, including failure/restart/expired-lease/concurrent-worker coverage.
The isolated lab typecheck, 35 fake SDK/DOM tests and Vite build also pass.

The [local SFU fixture](../../apps/voice-lab/testing/README.md) now runs real
LiveKit 1.13.11 with three synthetic Opus peers. Each received two remote tracks
for 10,003 ms (1,206 / 1,202 / 1,188 nonempty RTP packets); stopping sharing
disconnected all three. This verifies transport and closure, not acoustic quality,
packet-loss rate or latency. The deliberate cached-token rejoin **succeeded**.
Accordingly the strict production-revocation gate exits nonzero; functional
test success is not reported as release approval. Cached/refreshed-token exclusion,
logout reconciliation, native integration/loopback exclusion and physical audio
trials remain. Product version/public release stay 2.3.3; no release was created.

Phase 5b4 (2026-11-07) supersedes the cached-token blocker above. Enabled
configuration now requires a Sanser admission gateway with a loopback-only SFU
signaling upstream. It validates current room revision, login, device presence
and membership on join and while signaling is active. Logout/offline
reconciliation retires invalid rounds; terminal Leave messages prevent reconnect
loops. With this topology, both the original token and an actual SFU-refreshed
token failed rejoin in the real three-peer test. All three peers received two
remote Opus tracks over ten seconds, and all disconnected after group stop.
The strict production-revocation fixture now passes. This assumes the upstream
SFU HTTP/WS listener remains private; exposing it separately bypasses admission.

Desktop now consumes the shared Lab session/LiveKit adapter in a restricted
local floating voice window. The host combines Stop sharing with voice; viewers
get voice controls only. Before joining, a scoped native ACK confirms its old
mic/player has stopped and shared system sound is paused. Closing restores shared
sound without restoring microphone consent. Enhanced voice currently owns one
desktop session at a time. A 25-second startup watchdog (checked every two
seconds) releases the pause if the webview never confirms a connected room.

Validation: 38 server unit tests, 12 isolated PostgreSQL API tests, the real
three-peer SFU revocation gate, desktop Rust tests, 295 desktop UI tests before
the startup-watchdog increment and six focused voice-window tests afterward,
strict typecheck/lint/build/Clippy, and synthetic native audio/video gate tests.
These are transport, authorization and lifecycle results, not acoustic approval.
The user cannot currently run the physical Windows/Mac trial. Windows native
runtime, WebView microphone/output/focus, 2–3 speakers, jack changes and WAN SFU
deployment remain unverified. See [deployment and acceptance](../livekit-voice-gateway.md).
The release hold remains; product version/public release are still 2.3.3.

Phase 5b5 (2026-11-07) fixes mic acquisition/publication hanging indefinitely,
with a 15-second deadline and cleanup of late grants/publish completions. Old
stopped-mic events cannot mute a replacement; browser device changes retry
playback without restoring mic consent. Successfully closed rooms now expire
after 30 days in bounded batches; pending/live/leased work is retained.

The real SFU test now keeps heartbeats and the production closure worker active
throughout. A 120,045 ms three-peer synthetic Opus sample received two tracks
per peer (12,170 / 12,128 / 12,127 packets), with maximum userspace RTP receipt
gaps of 63.487 / 63.333 / 63.312 ms and no per-track three-second stall. A separate
short run still checks original-token denial before expiry. The updated 13-test
PostgreSQL suite and local revocation fixture pass. These results do not measure
decoding, speaker output, echo cancellation or audible latency. Public deployment
templates are prepared in deploy/voice; there is still no public SFU or physical
two-machine acoustic validation. The hold remains, version 2.3.3 unchanged.
