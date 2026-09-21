# Optional remembered sign-in

- **Keep me signed in** is unchecked by default on the existing login/register
  form. The user enters their account password once, in that form.
- Unchecked: access/refresh tokens remain in process memory. No credential is
  written for that login, and a previously saved session known to the app is
  removed when the unchecked login succeeds. Closing and reopening requires
  login again.
- Checked: the app stores tokens, normalized server URL and last successful
  activity time as one secure credential. It does not store the password.
- Reopening within seven days refreshes the saved session and opens the account.
  Each successful return or authenticated API activity renews the inactivity
  window. Seven days is measured from last use, not initial registration.
- At seven days of inactivity, the app requires login again. An open app checks
  once a minute and on focus/visibility changes, using the regular sign-out path
  to stop engines and clear presence. Failed network requests do not extend the
  window, and temporary outages preserve otherwise valid saved credentials for
  retry. Activity timestamps are written at most once per minute, allowing a
  conservative margin of up to a minute when the process exits between writes.
- Tokens cannot be restored to another server. Invalid records, future timestamps
  beyond the clock-skew allowance and expired records are cleared. Older entries
  without activity metadata require one fresh login.

The Rust keyring dependency previously had neither `apple-native` nor
`windows-native` enabled. Keyring 3 falls back to a mock backend without these
features. Both platform backends are now explicitly enabled. macOS may request
OS-level permission to access its Keychain, particularly for a newly built or
unsigned app; the application cannot bypass that permission. There is no separate
Sanser password-to-save dialog.

Server access and refresh authentication now enforce seven idle days using
`auth_sessions.last_seen_at`, with access activity written at most once per minute.
This uses existing schema columns. The API advertises `session-idle-7d` in
health/readiness after deployment. Desktop enforcement is already implemented
locally; server enforcement requires deploying this server code. Nothing here
deploys to Render or modifies production data.

## Verification

Frontend regression coverage includes checked/unchecked submissions, memory-only
login, restoration after module restart, the exact seven-day boundary, returning
before the boundary, activity renewal, offline recovery, rejected tokens, wrong
server/invalid records, secure-storage failure and late replies after logout.
The latest frontend run passed all 86 tests, type checking, lint and the production
build. The default Rust workspace run passed 156 tests; it reported six database
tests and the opt-in credential persistence test as ignored. The latter was also
run explicitly and passed as described below.

Rust tests assert that the native credential backend is selected and that the
server idle boundary excludes seven days exactly. An opt-in native integration
test writes a synthetic credential in one process, reads it in a second process,
then deletes it. It uses a dedicated test service and random identifier, never
the user's saved credential. That test passed on this Mac:

```sh
node scripts/run-cargo.mjs test -p sanser-desktop --test credential_persistence -- --include-ignored
```

The database-backed idle-policy test is compiled but ignored unless explicitly
run against a dedicated `TEST_DATABASE_URL`. Windows credential persistence has
not been executed on a physical Windows machine. No real account was used to
verify automatic sign-in in this run; authentication tests mock API responses.
