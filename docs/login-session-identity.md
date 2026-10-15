# Login sessions and server configuration

The Account page lists authentication sessions, not online computers. Versions up
to 2.2.4 omitted `deviceName` and `platform` at sign-in, so the server recorded
`Sanser Desktop` / `unknown`. Each password login created another session; saved
sign-in refreshes already reused the existing session.

Desktop password sign-in and registration now include the actual computer name,
OS/architecture, and a persistent random `installationId`. The native identifier
lives in the app configuration directory (`login-device.json`), contains no
credentials, survives logout/updates, and does not enable remembered sign-in.
Passwords are never saved. The existing optional seven-day inactivity policy is
unchanged.

Migration `0008_login_installation.sql` adds an optional installation identifier.
After verifying the password, a transaction serializes sign-ins for the account,
revokes prior sessions belonging to that same installation, and issues fresh
tokens. Session replacement is scoped to `(user_id, installation_id)`, never the
computer name or OS. Simultaneous logins leave only the last session valid.
Other installations and accounts keep their sessions. Existing anonymous sessions
cannot be attributed safely and remain available for individual sign-out in a
collapsed section on the Account page.

Deploy the server with its migration and update the desktop app to enable session
replacement. Older clients remain accepted. A new app talking to an old server
retries only an explicit unknown-`installationId` validation error, omitting that
field while retaining the computer labels; session replacement then requires the
server update. Authentication and network failures never trigger that fallback.

Normal sign-in uses the configured server automatically. The server URL field is
available through **Use a different server**, remains visible for a previously
selected custom server, and is required when no default exists. Hiding the field
does not deploy or switch servers. `https://sanser.onrender.com` remains the
default authentication/signaling server unless explicitly reconfigured; its
presence alone does not identify the selected media route as Direct or Relay.

Validation includes frontend identity/compatibility/session display tests, Rust
unit tests, and the opt-in PostgreSQL API test
`password_login_replaces_only_the_same_installation_after_verification`, which
covers concurrent replacement, invalid passwords, old token rejection, identical
computer labels, account isolation, and legacy clients. Run database tests only
against a dedicated `TEST_DATABASE_URL`:

```sh
node scripts/run-cargo.mjs test --locked -p sanser-server --test api -- --ignored
```
