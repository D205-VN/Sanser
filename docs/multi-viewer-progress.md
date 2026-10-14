# Multi-viewer sharing: implementation progress

This work adds view-only sharing for up to three viewers. Existing single-peer
remote-control sessions remain separate. Mac hosts and Windows clients already
exist; this project reuses their SNV2/H.264 engines.

| Step | Deliverable | Current state |
| --- | --- | --- |
| 1 | Sharing data model, ownership, invitation consent and stop API | Implemented; unit tests pass. Dedicated PostgreSQL integration test remains to run in step 5. |
| 2 | Server-issued view-only permission, native client input suppression, host enforcement | Implemented; native loopback and Rust/TypeScript tests pass. Real-device validation remains in step 5. |
| 3 | One capture/encoder feeding multiple independent authenticated peers | Pending |
| 4 | Invite/join/remove/stop UI and persistent sharing indicator | Pending |
| 5 | Database concurrency/migration tests, Windows/macOS CI, 2–3-machine validation and release | Pending |

**Two implementation steps completed; three remain. Multi-viewer media is not
available yet and no new multi-viewer installer has been published.**

## Control-plane contract (step 1)

Migration `0006_sharing_permissions.sql` adds `sharing_groups`, `sharing_members`
and immutable connection-session `access_mode` (no permission-update endpoint).
Tenant ownership uses composite foreign keys. One open group per host, one
reservation per viewer and occupied slots 1–3 have database uniqueness
constraints. Transactions serialize invitations on the group row and competing
viewer/session requests on device rows. Normal connection requests respect
sharing reservations. Stop removes reservations atomically; preparation expires
after 15 minutes without explicit invitation activity or when the host goes offline.

Authenticated `/api/v2/sharing` endpoints:

- `GET /`: current preparation groups in this account.
- `POST /`: `{hostDeviceId, maxViewers: 1..3}`.
- `GET /{id}`: group and viewer invitations.
- `POST /{id}/invite`: `{viewerDeviceId}`; always view-only.
- `POST /{id}/members/{member}/respond`: `{viewerDeviceId, accept}`.
- `POST /{id}/stop`: release viewer reservations.

`preparing` and invitation `accepted` describe preparation/consent only. They
do not start a native engine, mark a device as streaming, or issue media
credentials. There is deliberately no group-media start endpoint until step 3
can service its promises. The sharing UI will be connected in step 4.

## Native permission contract (step 2)

Normal session creation can request `accessMode: "view-only"`; missing means
the existing `control` behavior. View-only sessions require version 2.1.22 or
newer on both devices, so older hosts cannot silently ignore the new permission.
The code is being prepared for that release; the current published 2.1.21 does
not expose it. Native credentials carry the immutable permission and use a
permission-specific derivation; host launch applies `--disable-input` even when
the host's general remote-input preference is enabled.

Both legacy Mac and SNV2 clients honor disabled input. The legacy Mac sender
rejects keyboard, mouse, clipboard and gamepad messages before enqueueing,
keeps control ping/stats/NACK/keyframe requests, avoids relative mouse grab and
keeps the local cursor visible. Hosts enforce input denial on receipt. SNV2
tests send authenticated malicious keyboard/mouse packets directly to a
view-only host and verify that keyframe requests still work.

## Validation and remaining integration

Rust library tests, native CTest, Svelte checking/lint and frontend tests have
been run locally. `sharing_capacity_consent_ownership_and_stop_are_transactional`
in `apps/server/tests/api.rs` races four invitations, verifies exactly three
succeed, checks account ownership/consent and confirms stop releases reservations.
It is opt-in and requires an isolated `TEST_DATABASE_URL`; production
`DATABASE_URL` must never be used for this test.

Remaining native integration needs one encoder output fanned out through
per-viewer Peer objects, each with its own keys, sequence/replay windows,
feedback, repair and send queue. Joining/reconnecting a viewer must request a
fresh keyframe without restarting the other viewers. Per-session P2P/relay
negotiation and native lifecycle must be expanded beyond their current single
reservation/single host session behavior before enabling the UI.

Real-device tests must measure total upload, each viewer's P50/P95 RTT and frame
queue under load. Existing ~200 ms path latency remains a separate unresolved
measurement; broadcasting is not a fix for it.
