# Guest access (support sessions)

A person without a Sanser account can view, and if the host allows it control,
a host computer for one session. Typical use: helping a relative.

## Flow

1. **Host** (signed in, host computer online): Host page → Support session →
   *Create support code*. `POST /api/v2/guest/grants {hostDeviceId}` returns a
   9-digit access code (`123 456 789`) and an 8-character password, shown once.
   A new code replaces any unused one for that computer.
2. **Guest**: opens Sanser, Welcome → *Connect with a code*, enters code,
   password and a name. `POST /api/v2/guest/redeem` (no account).
3. The server checks the code and password, creates a temporary **guest device**
   and a **pending, view-only** connection session inside the host account, marks
   the code used, and returns a **guest token** plus the session.
4. **Host** sees "Guest: name" in Connection requests and must click *Accept*
   (optionally ticking *Allow control of keyboard and mouse*). The accept call
   must carry `{"guestConfirmed": true}`; otherwise the server refuses it.
5. The usual session flow runs: native-ready, credentials, signalling and relay.
6. The session ends when either side disconnects, the host rejects it or
   revokes the code (`POST /api/v2/guest/grants/{id}/revoke`), the guest device
   goes offline, or nobody answers within 2 minutes. The guest token and device
   are revoked at the same time.

## Limits

| Limit | Value |
| --- | --- |
| Code lifetime | 10 minutes or first successful use |
| Wrong passwords per code | 5, then the code is revoked |
| Redeem attempts | 20 per client address per 10 minutes, 600 per minute server-wide |
| Unanswered guest request | expires after 2 minutes |
| Guest token | at most 4 hours, and ends with the session |
| Default access | view-only; control only if the host ticks it when accepting |

## Threat model

| Threat | Defence |
| --- | --- |
| Guessing a code | 10⁹ codes, only a handful live, 10-minute life, per-address and global limits. A code alone is useless without the password. |
| Guessing the password | 32⁸ ≈ 2⁴⁰ passwords, 5 tries per code then the code is burnt; Argon2id hash. |
| Probing which codes exist | Unknown code, wrong password, expired, revoked and used codes all return the same 422 message; unknown codes still pay an Argon2 verification. |
| Database leak | Codes are stored only as an HMAC keyed with the server secret; passwords as Argon2id hashes; tokens as SHA-256 digests. |
| Reusing a code | Redeeming sets `redeemed_at` under a row lock; a second redemption is refused. |
| A guest reaching account data | Guest tokens carry `scope = 'guest'`. The default `AuthContext` extractor rejects them (403), so every account, device, sharing, session-creation and code endpoint is closed. Only `SessionAuth` endpoints accept them, and those check the token's own session and device: get session, credentials, native-ready, disconnect, device heartbeat/offline, ICE, signalling and relay sockets. The account event stream is closed to guests. |
| A guest accepting itself or getting control | Accept is account-only; guests start view-only, and the access mode is bound into the native session credential as for other view-only sessions. |
| Auto-accept | The desktop never auto-accepts a guest (`trustedPendingSession` skips them) and the server refuses any accept that lacks `guestConfirmed`, which also protects hosts running older app versions. |
| Guest device leaking into the account | Guest devices have `guest_grant_id`; they are excluded from device lists, `fetch_owned`, session creation, sharing and registration. |
| Lingering access | Ending the session revokes the grant, the guest token and marks the device offline; cleanup also catches sessions ended by other paths within a minute. |

Not covered: a host who shares the code with the wrong person, social
engineering, and per-address limits behind a proxy (all clients may share one
address, so the global limit is the real ceiling there).

## Operations

- Migration `0014_guest_access.sql` (new table, new columns on `devices`,
  `connection_sessions`, `auth_sessions`).
- No new environment variables. Codes are keyed with `SESSION_CREDENTIAL_KEY`;
  rotating it invalidates codes created in the last 10 minutes only.
- Guest grants and devices are kept 30 days, like ended sessions.
