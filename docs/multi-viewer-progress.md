# Multi-viewer sharing: implementation progress

This work adds view-only sharing for up to three viewers. Existing single-peer
remote-control sessions remain separate. Mac hosts and Windows clients already
exist; this project reuses their SNV2/H.264 engines.

| Step | Deliverable | Current state |
| --- | --- | --- |
| 1 | Sharing data model, ownership, invitation consent and stop API | Implemented; unit tests pass. Dedicated PostgreSQL integration test remains to run in step 5. |
| 2 | Server-issued view-only permission, native client input suppression, host enforcement | Implemented; native loopback and Rust/TypeScript tests pass. Real-device validation remains in step 5. |
| 3 | One capture/encoder feeding multiple independent authenticated peers | Native core and desktop command bridge implemented; application/session integration belongs to step 4. |
| 4 | Invite/join/remove/stop UI and persistent sharing indicator | Pending |
| 5 | Database concurrency/migration tests, Windows/macOS CI, 2–3-machine validation and release | Pending |

**Three implementation steps completed at the component level; two integration
and validation steps remain. Multi-viewer sharing is not available from the UI
yet and no new multi-viewer installer has been published.**

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

## Native broadcaster and command bridge (step 3)

`native/desktop/broadcast.*` prepares one encoded payload and hands shared
immutable storage to up to three independent Peer workers. Each viewer has its
own authenticated credential, sequence/replay windows, UDP socket, bounded
latest-frame delivery slot, pacing/send queues and repair cache. A failed/slow
worker does not block the encoder callback or another viewer's sender. Input
is disabled both in Peer and in the sharing launch contract; no injector
callback is installed for sharing peers.

Windows uses one DesktopDuplicator/MF encoder, macOS one ScreenCaptureKit/
VideoToolbox encoder. Late join/reconnect requests a coalesced fresh keyframe;
an idle screen can be refreshed from the most recent cached capture. Removing
one viewer keeps the capture/encoder process alive, including when the last
viewer leaves. Stopping the group joins all peer workers.

SNV2 repair now requests missing fragment indices promptly, retains a bounded
150 ms/16-frame/8 MiB per-viewer cache, and sends repairs with fresh authenticated
sequences. Repair has a lower transmit priority than current video. Replayed or
foreign-key NACKs cannot access a cache. Repair remains best effort: a newer
completed frame can supersede an older partial frame; fresh IDR recovery is
still necessary for lost codec references.

The desktop bridge launches `--snv2 --share-host --share-peer-id <viewer UUID>`
with `--disable-input`. Add/remove commands use a bounded stdin protocol and
correlated native acknowledgements. Tokens are not placed in argv or echoed in
acknowledgements; temporary Rust command strings are zeroized. A lost command
acknowledgement blocks further membership changes until restart without killing
other viewers. Acknowledgement means the peer was added/removed, not that its
handshake or video succeeded. Older engines must advertise `multiViewerHost`
before this launch is allowed.

Local validation includes three authenticated UDP viewers, isolated disconnect,
a constrained viewer dropping stale video while healthy peers continue, cache
authentication/replay/expiry, and a real single VideoToolbox encoder feeding
three decoders with idle late join and member removal. This is loopback codec
validation; it does not replace real Windows GPU/screen/network testing.

## Remaining integration (steps 4–5)

The command bridge currently accepts Direct IPv4/SNV2/H.264 sharing without
audio. The API still exposes preparation/consent only. Step 4 must bind accepted
members to individual media sessions/credentials, negotiate each viewer,
integrate relay lifecycle without replacing other bridges, and show
handshake/first-frame status rather than treating command acknowledgement as
connected. Current P2P/relay reservation managers still represent one attempt
per engine; parallel viewer negotiations need separate reservations or explicit
serialization. UI invitations, join/leave/remove/stop and a persistent sharing
indicator remain to implement. Shared bitrate adaptation from the slowest
viewer's delivery feedback is also pending; the native core currently uses the
configured encoder bitrate and isolates each sender's pacing/drop behavior.

Real-device tests must measure total upload, each viewer's P50/P95 RTT and frame
queue under load. Existing ~200 ms path latency remains a separate unresolved
measurement; broadcasting is not a fix for it.
