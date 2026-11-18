# Two-step verification (TOTP)

Accounts can require a 6-digit code from an authenticator app (RFC 6238:
HMAC-SHA1, 30-second steps, 6 digits) in addition to the password.

## Flows

| Step | Endpoint | Notes |
| --- | --- | --- |
| Status | `GET /api/v2/account/two-factor` | `enabled`, `recoveryCodesRemaining` |
| Start setup | `POST /api/v2/account/two-factor/enroll` `{password}` | Returns the base32 secret and `otpauth://` URI once. Unconfirmed setups are discarded after 15 minutes. |
| Confirm | `POST /api/v2/account/two-factor/confirm` `{code}` | Turns 2FA on, returns 10 recovery codes once, and signs out every *other* account session (their refresh tokens are revoked). |
| Turn off | `POST /api/v2/account/two-factor/disable` `{password, code \| recoveryCode}` | Deletes the secret, recovery codes and open challenges. |
| New recovery codes | `POST /api/v2/account/two-factor/recovery-codes` `{password, code \| recoveryCode}` | Replaces all codes; old ones stop working. |
| Sign in | `POST /api/v2/auth/login` | With 2FA on, a correct password returns **401** `two_factor_required` with `challengeToken` and `challengeExpiresAt` instead of tokens. |
| Second step | `POST /api/v2/auth/login/two-factor` `{challengeToken, code \| recoveryCode}` | Returns the usual `AuthTokens`. |

The desktop app shows the QR code rendered locally (`apps/desktop/src/lib/qr.ts`);
the secret never goes to a QR service. Settings → Security holds the card; the
Welcome screen asks for the code after the password.

## Older clients

Accounts without 2FA sign in exactly as before. An older client signing in to
an account with 2FA gets a 401 whose message reads "Two-step verification is
turned on for this account. Update Sanser to the latest version to sign in."
It never receives tokens.

## Storage

- **TOTP secret**: XChaCha20-Poly1305 envelope (`tfa1.<key id>.<hex nonce+ciphertext>`)
  with the account id as associated data, so a ciphertext cannot be moved to
  another account. Key: `TWO_FACTOR_ENCRYPTION_KEY` (base64, 32–128 bytes) when
  set (key id `k`), otherwise HMAC-SHA256 of `SESSION_CREDENTIAL_KEY` with a fixed
  label (key id `d`). Each envelope records which key sealed it.
  **Rotating `SESSION_CREDENTIAL_KEY` breaks `d` envelopes** (those users cannot
  pass the second step and need recovery codes or an operator reset), so set a
  dedicated `TWO_FACTOR_ENCRYPTION_KEY` before many people enrol and keep it.
- **Recovery codes**: 10 per account, `XXXXX-XXXXX` (50 bits), Argon2id hashes
  like passwords, single use (`used_at`).
- **Challenges**: SHA-256 digest of a random bearer token, 5-minute lifetime,
  bound to the account and to the device name/platform/installation id of the
  password step.

## Threat model and limits

| Threat | Defence |
| --- | --- |
| Stolen password | Login stops at the challenge; tokens need a valid code. |
| Guessing codes | 5 attempts per challenge (counted atomically before checking), then a new password login is needed; 10 code attempts per account per 10 minutes across challenges, confirm, disable and recovery-code changes; password login is still limited per email. |
| Replaying a seen code | The highest accepted step is stored; codes for that step or earlier are refused, so a code works once. |
| Clock drift | ±1 step (±30 s) accepted. |
| Password-only sessions that predate 2FA | Turning 2FA on revokes all other account sessions and their refresh tokens; refresh can therefore never mint tokens for a session that skipped the second step. |
| Database leak | Secrets are encrypted with a key held outside the database; recovery codes are Argon2id hashes; challenges are digests. |
| Cross-account use | A challenge only accepts its own account's code or recovery code. |
| Turning 2FA off from a hijacked session | Requires the password and a valid code or recovery code. |

Not covered: phishing proxies that relay a fresh code in real time, malware on
the signed-in computer, and in-memory rate limits resetting when the server
restarts (they are per process). Changing the password does not require a code.

## Operations

- Migration `0013_two_factor.sql` must be applied (the server runs migrations at
  start-up).
- Recommended: set `TWO_FACTOR_ENCRYPTION_KEY` (e.g. `openssl rand -base64 32`).
- Lost authenticator and recovery codes: an operator can delete the account's
  rows in `user_two_factor` and `two_factor_recovery_codes` after verifying the
  person by other means.
