# Multi-viewer sharing: implementation progress

All development remains on `Version-2.0.0`. The feature shares a Mac or Windows
screen with up to three view-only computers in the same account, using the
existing platform-native SNV2/H.264 capture, encoder and decoder backends.

| Step | Deliverable | Current state |
| --- | --- | --- |
| 1 | Sharing model, ownership, invitation consent and stop API | Implemented, including isolated PostgreSQL concurrency tests. |
| 2 | View-only credentials, client input suppression and host enforcement | Implemented; native authentication/permission tests pass. |
| 3 | One capture/encoder delivering to independent authenticated peers | Implemented; three-viewer UDP and real VideoToolbox codec loopback tests pass. |
| 4 | Invite/join/remove/stop UI, independent transport lifecycle and visible sharing indicator | Implemented, including viewer retries, logout cleanup and shared bitrate feedback. |
| 5 | Database/Windows/macOS CI, installers and real 2–3-computer validation | CI passed all nine jobs; Mac/Windows Preview installers are published. Physical 2–3-computer and network validation remains. |

**Four implementation steps are complete. Step 5 has passed automated checks
and Preview packaging, with real-device testing still open.** Loopback tests do
not prove physical Windows capture, GPU compatibility or WAN performance.

- [Verified CI: 9/9 jobs](https://github.com/D205-VN/Sanser/actions/runs/38561115391).
- [Preview build/publication: 4/4 jobs](https://github.com/D205-VN/Sanser/actions/runs/38561849882).
- [Download 2.1.22 Preview](https://github.com/D205-VN/Sanser/releases/tag/v2.1.22).

The Preview tag points to `af405fca251448943890b2bfbdec6a52608e08dc`, the code
commit verified by those checks. Later documentation updates remain on the same
development branch.

## Using the feature

Update the server and all participating desktops to 2.1.22 or newer. The server
must apply migrations 0006–0007 and advertise `sharing-media`; the native host
must advertise `multiViewerHost`. An older server/sidecar cannot enable sharing.

On the host, go online, select **Start sharing**, choose a computer and select
**Invite**. Each invited computer explicitly selects **Join**. **Remove** stops
one viewer; **Stop sharing** stops the group. A group left by an earlier app
lifecycle is paused and must be stopped before creating a new group. Logging
out or taking the host offline stops its sharing process.

The application header shows sharing across pages. While streaming, a native
floating indicator also remains visible on the host screen and offers **Stop**.
The viewer count means decoded first frames reported for the current session
generation, not invitation acceptance or native command acknowledgements.

## Server lifecycle and permissions

Migration `0006_sharing_permissions.sql` creates groups/members and immutable
session access mode. Migration `0007_sharing_media.sql` associates accepted
members with individual native media sessions and first-frame status. Composite
foreign keys enforce account ownership. Database constraints enforce one open
group per host, occupied slots 1–3 and one active reservation per viewer.

Authenticated `/api/v2/sharing` endpoints:

- `GET /` and `GET /{id}`: this account's open groups and invitations.
- `POST /`: `{hostDeviceId, maxViewers, networkMode, qualityProfile}`.
- `POST /{id}/invite`: `{viewerDeviceId}`; always view-only.
- `POST /{id}/members/{member}/respond`: `{viewerDeviceId, accept}`.
- `POST /{id}/members/{member}/remove`: end only that viewer's media session.
- `POST /{id}/heartbeat`: `{hostDeviceId}`; keep the explicit group alive.
- `POST /{id}/stop`: end the group's sessions and release reservations.

Accepting creates a distinct view-only session. Client readiness advances its
server-issued negotiation epoch; its credentials always select SNV2/H.264,
including Windows-to-Mac sharing. `native-started` accepts only the correct
viewer/session/current epoch after decoding. Group row locks serialize this
report with retry, member removal and stop, preventing late reports from
reviving watching status. Idle groups expire after 15 minutes or host offline.

View-only credentials use a permission-specific HMAC derivation. Client engines
suppress mouse, keyboard, clipboard and gamepad input, preserve control/repair,
and keep the local cursor visible. Host peers independently reject authenticated
input packets. The host's general input preference cannot grant viewers control.

## Native broadcaster

`native/desktop/broadcast.*` shares one immutable encoded payload among up to
three independent Peer workers. Each has its own credential, packet/replay
sequence, UDP socket, bounded latest-frame delivery slot, pacing and repair
cache. A slow or failed viewer does not block the capture/encoder callback.

Windows uses Desktop Duplication/Media Foundation; Mac uses ScreenCaptureKit/
VideoToolbox. Late join requests a coalesced fresh keyframe, including an idle
screen refreshed from the last capture. Removing a viewer keeps the encoder
alive; group stop joins all workers. GPU-device changes invalidate cached capture
and recreate Windows encoding resources.

Missing fragments trigger bounded authenticated NACKs. Each peer retains repair
payloads for at most 150 ms, 16 frames and 8 MiB; shared encoded storage avoids
copying the same payload per viewer. Repairs use fresh packet sequences and
lower transmit priority than current video. Replayed/foreign-key requests cannot
read another viewer's cache. IDR recovery remains necessary after lost references.

The stdin command bridge correlates bounded add/remove acknowledgements without
putting credentials in argv or logs. A lost acknowledgement blocks further
membership changes until restart. Initial acknowledgement follows host startup;
it does not count as a received video frame.

Authenticated per-viewer delivery feedback adjusts a shared encoder budget to
the lowest active peer budget while preserving FPS. Idle traffic and a single
bad feedback sample do not lower bitrate; repeated delivery deficits reduce it
within bounds and sustained healthy delivery recovers it gradually. Feedback is
rate-limited. This is a conservative controller, not a completed GCC/BBR estimator.

## Desktop and transport lifecycle

Sharing uses per-session P2P reservations, signaling receivers and relay bridges.
Auto negotiates Direct first, then the existing authenticated UDP relay or WSS
fallback. Each viewer can reconnect with a fresh readiness epoch independently.
Removing one peer does not stop other peers or a local client connection.

Pending relay negotiations count toward the three-viewer limit. Stop/replacement
invalidates pending reservations, so a late negotiation cannot install a bridge.
Selected Direct ports are released if launch is cancelled. Native encoder
creation is serialized while peer negotiations proceed independently.

Logout cleanup completes before reinitialization. A stopped group cannot restart
from a stale poll response, even if the server stop request fails. The native
first-decoded-frame event is distinct from overlay/control traffic.

## Validation and remaining work

Local validation passed Svelte checking/lint, 156 frontend tests, Rust workspace
tests and 12 macOS native CTests. Tests cover single encoder/three peers, independent
relay fallback/removal, fresh viewer retry, shutdown/reinitialization, cancelled
Direct reservations, repair authentication/expiry and real VideoToolbox idle late
join. Windows-native compilation and all 12 loopback/codec tests passed in GitHub CI.

The dedicated PostgreSQL CI job runs eight API integration tests, including
migration application, racing four invitations for three slots, consent,
account isolation, individual credentials, stale first-frame rejection,
member removal and group-stop reservation release. Local runs without a dedicated
`TEST_DATABASE_URL` explicitly skip these tests; production databases are never
used by this fixture.

The `v2.1.22` workflow published a GitHub **Preview prerelease** with nine assets:
Mac DMG/ZIP/updater bundle, Windows installer/portable executable, updater
signatures, platform manifest and SHA-256 checksums. The previous 2.1.21 stable
release remains latest. See `docs/releases/v2.1.22.md` for scope and use.

Still required on real devices:

- Windows and Mac hosts with two/three viewers: join late, remove, stop and retry.
- Windows GPU capture/encoding and the native floating Stop indicator.
- Direct LAN, WAN and relay: measure per-viewer P50/P95 RTT, input/frame queue,
  upload throughput and whether a constrained viewer affects healthy viewers.
- Public UDP relay deployment/validation on a server with actual UDP ingress.

The earlier approximately 200 ms route latency is unresolved by this feature;
no 5–15 ms end-to-end latency or broad hardware compatibility is claimed.
Audio sharing, mobile viewers and a privileged Windows Secure Desktop service
are outside this Preview.
