# Sanser Voice Lab — phase 5b prototype

An isolated browser voice experiment using **livekit-client 2.23.0** (pinned in
its own package/lockfile). The desktop enhanced-voice window now reuses this
module's session/SDK adapter, with a pinned desktop runtime dependency. The
browser lab remains an independent measurement tool, not a Flutter mobile port.
No new Sanser release has been published.

## Run

From the repository root, using Node compatible with the existing desktop tools:

```sh
npm --prefix apps/voice-lab ci --workspaces=false --ignore-scripts
npm --prefix apps/voice-lab run dev
```

Open **http://127.0.0.1:1431** locally. The development server binds loopback only;
run a local copy on each computer. A shared LiveKit server carries the voice.
The page is responsive, but this does not establish native iPhone/Android support.

### Join through a Sanser account (phase 5b1)

The lab now supports an existing Sanser account and sharing group. This is
**experimental admission**, disabled by default on the Sanser server. It does
not silently replace native voice. A configured gateway enables the desktop
**Join enhanced voice** action; mobile integration is separate.

The server operator supplies these settings on their test server (never in the
browser, repository, exported report or a desktop/mobile build):

```dotenv
VOICE_LAB_EXPERIMENTAL_ENABLED=true
VOICE_LAB_LIVEKIT_URL=wss://your-livekit-server
VOICE_LAB_LIVEKIT_API_KEY=<server API key>
VOICE_LAB_LIVEKIT_API_SECRET=<server API secret of at least 32 bytes>
```

Keep the existing explicit `ALLOWED_ORIGINS`, appending
`http://127.0.0.1:1431` for this loopback lab. Do not use a wildcard. Production
credentials/configuration have not been changed or provisioned by this work.
The example's blank values are deliberately unusable.

1. On the real Sanser host, start sharing; viewers accept their invitations.
   Leave native Sanser running so registered devices continue heartbeating.
2. In Voice Lab, enter the **trusted Sanser API URL**, email and password. This
   creates a separate in-memory browser login, not a new registered computer.
   Its random installation ID never replaces the native app's installation.
   Password clears on submit. No login/join tokens are persisted or exported.
3. Choose the host or accepted viewer role corresponding to this computer.
   Current labels show the short registered device and sharing IDs. This follows
   Sanser's account authority: it does not prove browser-to-physical-device binding.
   All participants in this trial use account-owned devices, as existing sharing
   does; this is not cross-account invitation support. One lab per selected role.
4. Choose **Tham gia · chỉ nghe**, then explicitly **Mở mic** if wanted. The
   server creates the room/identity mapping and credential; no JWT paste needed.
5. **Rời phòng** immediately stops voice and cancels admission in progress.
   **Đăng xuất** also forgets the lab login and attempts to revoke that Sanser
   login. Page exit also forgets credentials and attempts logout with `keepalive`.
   Network failure/page close can prevent this best-effort logout; Sanser's
   existing login expiration/account-session revocation still applies. The lab
   does not auto-refresh access tokens; reauthenticate when the login expires.

`POST /api/v2/sharing/{id}/voice-lab/credentials` accepts only `deviceId` and
requires the existing account bearer. A transaction first locks the owned group,
then checks membership in a fresh statement: an admission waiting on removal
cannot combine the new room revision with the previous membership. It checks the
owned active/preparing group, fresh online host/participant, and for a viewer,
accepted membership plus its matching live accepted view-only session. Foreign
groups/devices, unaccepted/removed viewers, stale devices and stopped groups
cannot obtain a new grant. Unknown JSON fields (including caller-selected rooms)
are rejected. A per-account/device limiter allows 12 requests/minute.

The returned HS256 JWT permits only microphone publishing/subscription in one
opaque room, no video/data/admin/recording/metadata permissions. Its initial
admission lifetime is 60 seconds. Room and identity use domain-separated HMACs;
no email, name or raw account/device/group ID enters those SFU fields. The
response is `Cache-Control: no-store`. API secrets stay server-side in zeroizing
storage, with no `Debug` implementation or credential logging.

#### Room revision fence (phase 5b2)

Migration `0009_voice_lab_room_revision.sql` maintains a monotonic revision per
sharing group. Removing/deleting an accepted member, changing its linked session
or identity, or stopping/reassigning a group changes the revision. Triggers cover
the normal routes and direct cleanup/cascading SQL paths. Removing an already
removed member and ordinary heartbeat/status writes are idempotent; accepting a
new viewer does not interrupt the existing round.

New credentials bind their `video.room` grant to that revision, via HMAC. A token
issued for an older round cannot authorize the **new** room. The opaque
`roomRevision` response field is required by the lab and never exported in stats.
When a membership check reports a different revision, the conforming client stops
mic/leaves the old room and asks for an explicit rejoin, with mic still off.
It does not silently move a live microphone to another room. A refreshed token
for the same revision does not interrupt the call.

The revision fence alone is not revocation: a self-hosted SFU can accept old
cached/refreshed tokens. Phase 5b4 now checks every signaling join/rejoin at the
Sanser gateway and closes invalid rounds. This requires **private SFU signaling**;
exposing the SFU port directly bypasses the gateway. TTL and client polling are
not substitutes for that boundary. See the deployment contract below.

#### Durable SFU room closure (phase 5b3)

Migration `0010_voice_lab_room_closures.sql` registers each issued room under the
same group lock used for admission. Revision changes and group deletion retire
registered rooms through database triggers. The persistent outbox survives API
restarts and deletion of the sharing group. It contains internal identifiers and
opaque room names, never JWTs or API secrets.

An opt-in background worker claims up to four due rooms with `SKIP LOCKED` and
30-second leases. It calls LiveKit `DeleteRoom` without holding database locks,
retries failed closures with bounded backoff, and reconciles expired leases.
Only the configured endpoint/key scope is eligible. Drain the queue using the
old configuration **before** changing the SFU endpoint or rotating its key/secret;
pending jobs for another scope are intentionally not sent to a new deployment.
Successfully closed/retired rows are retained for 30 days, then pruned in batches
of at most 100; admission mappings cascade away. Active, pending and leased rows
are preserved, including old-scope closure work.

The internal administration JWT never enters a browser response or log. The
HTTP client disables proxies/redirects, limits time/body size and accepts only
an authenticated empty JSON success or a Twirp `not_found` acknowledgement.
Remote endpoints require HTTPS. Disabling Voice Lab starts no worker or network
requests.

The initial direct-SFU test reproduced cached-token replay after `DeleteRoom`.
That evidence is retained. The current test goes through the enforced gateway
and rejects both the original token and an **actual SFU-refreshed token** after
room closure; three peers each receive two remote Opus tracks before closure.
See [the reproducible procedure and its limits](testing/README.md).

#### Enforced gateway and desktop handoff (phase 5b4)

Migration 0011 maps opaque room/identity to the login session and owned device.
Identity changes with the login session. The gateway verifies HS256, issuer,
expiry, exact microphone-only grants and fresh database authorization before
opening an upstream socket, and rechecks after handshake and periodically.
Membership/removal, offline devices, stopped groups and revoked/expired logins
reject admission even for refreshed SFU tokens. The closure worker reconciles
these invalid logins/devices and retires their round. A terminal LiveKit Leave
signal avoids an SDK reconnect loop if its socket check races room deletion.
Closure retries during an SFU/DB outage can delay existing-peer termination;
no instantaneous revocation claim is made.

The enabled server configuration requires an SFU WS listener on `127.0.0.1`
and a separate public Sanser WSS origin. Only Sanser's `/rtc` and `/rtc/v1`
(and their `/validate` routes) are exposed. Request traces omit query strings.
[Deployment and final physical-test checklist](../../docs/livekit-voice-gateway.md).

Desktop shows **Join enhanced voice** only when the authenticated server reports
a configured gateway. It hands a scoped room grant once to a local floating
window, never via URLs, broadcast events or disk. That window has only its
handoff/host-stop app commands and window close/drag permissions; it cannot read
account storage or launch engines. Join is listen-only; reconnect/Leave/late
capture callbacks cannot restore microphone consent. A host gets the combined
share/voice controls and Stop sharing; a viewer gets voice-only controls.

Before opening the window, the native engine acknowledges that its mic/player
has stopped and system-sound publication is paused. The native bar yields to the
new bar; closing it restores native sharing controls/sound. Old sidecars without
the explicit gate capability are rejected. Native session termination closes the
voice window. Capture permission, hardware quality, Windows runtime behavior and
long-session A/B still require real-device tests. Shared music is deliberately
paused during enhanced voice, rather than claiming browser loopback exclusion.
If its webview never confirms a connected room, a 25-second startup watchdog
closes the window on the next two-second check and restores native shared sound.
The experiment and release hold remain until those trials can be completed.

Phase 5b5 adds bounded mic capture/publication: after 15 seconds a stalled prompt
or publish fails with mic off, and late completions are stopped/unpublished.
Old stopped-track events cannot mute a replacement mic. A browser device change
retries playback without acquiring a microphone; autoplay denial remains visible.
Fake-device tests cover these cases; physical jack/driver recovery is not proven.
The real three-peer fixture now supports 10–600-second samples, fresh heartbeats
and active authorization checks throughout. A 120-second sample passes without
a per-track three-second stall. It tests RTP continuity, not acoustic quality.

### Manual endpoint/token trial

The collapsed advanced form still supports an operator-provided token, outside
the Sanser-account flow:

1. Obtain a trusted **LiveKit WSS endpoint** and a short-lived, room-scoped join
   token for each participant. Use unique opaque identities and the same room.
   The LiveKit administrator/backend creates tokens; API secrets stay there.
   A suitable grant enables `roomJoin`, `canSubscribe`, `canPublish`, restricts
   `canPublishSources` to `["microphone"]`, and disables `canPublishData`.
   Use a listen-only token if publishing is not wanted. Server-side permissions
   remain authoritative; the UI does not grant membership or mint tokens.
2. Enter the endpoint/token and choose **Tham gia · chỉ nghe**. The token field
   clears immediately and no microphone is opened. Use **Bấm để phát âm thanh**
   if browser autoplay requires another gesture.
3. Choose **Mở mic** explicitly. **Tắt mic**, **Tắt nghe** and **Rời phòng** stop
   owned microphone tracks. Hearing again does not reopen the microphone.
4. Network reconnection, a hidden/minimized tab, page close or hot reload ends
   this experimental call. Rejoin manually; microphone consent is never restored.
   Keep the page visible beside Sanser during a trial.
5. Export measurements using **Xuất số liệu**. This downloads numeric browser
   observations only, with up to 120 periodic samples.

For local server development, `ws://localhost:PORT` / `ws://127.0.0.1:PORT` are
accepted. Remote plaintext WebSocket, embedded URL credentials, query/hash token
URLs and malformed join tokens are rejected. No endpoint/token is saved in
storage, a URL, exported JSON or raw error UI. The SDK logger is silenced.
Use server-issued join tokens rather than credentials from a Sanser login.

The account token endpoint is implemented in source; no running Sanser/LiveKit
test deployment, API key/secret or real room has been provisioned. Manual tokens
do not receive the cooperative Sanser membership checks above.

## Controlled trial with Sanser

Start with 2–4 people and headphones. Use native voice in the native arm and
this lab in the LiveKit arm, with each arm's sound output and workload documented.
Run one voice system at a time; turn off native voice before joining this lab.

**Disable shared system sound on the host for both voice-only comparison arms.**
The native process-loopback exclusion cannot exclude this independent browser
from screen audio. Otherwise conversation can be recaptured and echoed through
Sanser even with headphones. Shared music + voice needs a later native/browser
exclusion integration; this browser prototype does not solve that case.

Use a trusted SFU and consenting participants. This lab uses the SDK's normal
WebRTC transport and does not implement room E2EE, recording, moderation or
membership revocation for the manual-token path. Account mode uses the gateway
above. Client limits are not server access control.
Configure room/grant limits server-side before a trial.

## Implementation and measurements

- `session.ts`: one attempt owns consent. Join timeout is 15 seconds. Leave,
  Mute/Deafen, capture/publish races, permission failures and obsolete callbacks
  cannot enable a later room's microphone. Pending capture is stopped when it
  resolves after cancellation; a browser permission request cannot be aborted
  programmatically before it returns a track.
- `livekit.ts`: no auto-subscribe. Only remote microphone audio is selected,
  with at most three tracks; camera/screen tracks are rejected. Mono speech
  capture requests browser echo cancellation, noise suppression and automatic
  gain control. Actual processing/device behavior still requires listening tests.
- `stats.ts`: bounded numeric projection of WebRTC audio observations. Receiver
  RTP jitter is a per-poll maximum. Sender remote-inbound RTT is **to the SFU**,
  not peer-to-peer or mouth-to-ear latency, and can be absent in listening mode.
  Jitter-buffer residence uses counter deltas, not lifetime division. First
  samples, resets and gaps over five seconds rebaseline affected counters.
  Missing/negative/unsupported fields stay absent; actual zero stays zero.
- Export strips RTC IDs, addresses, room/participant/device identities, tokens,
  SDP and media payloads. Browser concealed **samples** and native Opus concealed
  **packets** / PCM underflow **calls** have different units. Do not merge them
  into the native phase-5a comparator or invent a shared loss percentage.

This experiment creates separate `provider: livekit-browser` exports. The
[native A/B tool](../../docs/voice-performance-ab.md) does not automatically
compare this provider. Cross-provider measurement/consent/session integration
and real identical-device acoustic trials remain; no lower-latency or crackle fix
has been demonstrated by the prototype.

## Validation

```sh
npm --prefix apps/voice-lab run check
npm --prefix apps/voice-lab test
npm --prefix apps/voice-lab run build
```

35 tests use fake rooms/tracks, HTTP responses and DOM fixtures. They cover authenticated
selection/admission, late/cancelled login/grants, membership rejection, secret
clearing/nonpersistence, generic errors and request timeout, as well as listen-only Join,
explicit microphone capture, delayed capture/publish after cancel/Leave, prompt
serialization, deafen, reconnect failure, timeout/late connections, autoplay,
capacity, track source selection, ownership/cleanup, hidden-page teardown and
numeric stats privacy/units/retention. No test opens real audio hardware or joins
a LiveKit server. Mac Chromium desktop/narrow-layout screenshots were checked.
Build succeeds; Vite reports the SDK chunk above 500 kB (~157 kB gzip).

This does not establish live WebRTC interoperability, voice quality, real
multi-machine P50/P95, Windows browser/device behavior or iOS/Android support.
There is no configured LiveKit endpoint/token or local server in this workspace.
Server unit tests cover token format/signature/grants, identity scoping, endpoint
validation and disabled behavior. All **10 database API tests** now pass against
an isolated PostgreSQL 17.11 cluster, including actual account/membership/live
session/heartbeat/removal/stop admission. This caught and fixed an incorrect
integer decode of PostgreSQL's boolean online flags. Tests remain opt-in via a
dedicated `TEST_DATABASE_URL`; no production database was used. The temporary
test server bound only loopback and was stopped after the suite. `Config::test`
permits plaintext loopback tests; production Neon/TLS requirements are unchanged.
CI includes the database suite and a separate isolated Voice Lab check/test/build
job. Browser/SDK tests still use fakes; the DB run does not prove live SFU audio
or removal of an already-connected participant.

## Sources and notices

Implementation references (reviewed 2026-11-06):

- [LiveKit JS SDK](https://github.com/livekit/client-sdk-js), package types for 2.23.0.
- [Room connection / audio playback](https://docs.livekit.io/reference/client-sdk-js/classes/Room.html).
- [Selective track subscription](https://docs.livekit.io/transport/media/subscribe/).
- [Access tokens and grants](https://docs.livekit.io/frontends/reference/tokens-grants/).
- [RemoteAudioTrack stats](https://docs.livekit.io/reference/client-sdk-js/classes/RemoteAudioTrack.html).

LiveKit client SDK is Apache-2.0; its license is included under
[`public/licenses/livekit-client.txt`](public/licenses/livekit-client.txt) and
copied into the lab build. Dependency versions/integrity are recorded in the
isolated lockfile. No reference-project application code was copied into Sanser.
