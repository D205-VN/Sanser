# Email sign-in codes

Since 2.3.19 Sanser confirms account email addresses and asks for an emailed
code when a computer or phone signs in to an account for the first time. It
replaces the two-step verification (TOTP) of 2.3.17 and 2.3.18.

## Rules

- **Sign-up:** the server emails a 6-digit code to the address. The account is
  created only after the code is entered, so nobody can register an address
  they do not own. Until then, the address stays free.
- **Sign-in on a known computer:** a computer (installation) that has completed
  a sign-in to the account before signs in with the password alone, from any
  network or IP address.
- **Sign-in on a new computer:** after a correct password the server emails a
  code instead of returning tokens. Entering it signs in and remembers that
  computer for the account. A wrong password never sends an email.
- **Forgetting a computer:** signing out *another* computer from
  Settings → Account → login sessions forgets it, so it needs a code next time.
  Signing out on the computer itself keeps it remembered.
- Computers that were signed in before the upgrade are remembered
  automatically (migration `0017_email_verification.sql`).

A computer is identified by its random installation ID (a UUID created on first
run and kept in the OS keychain or secure storage). The server never returns
installation IDs to any client.

## Codes

| Property | Value |
| --- | --- |
| Format | 6 digits; spaces and dashes are ignored |
| Lifetime | 10 minutes |
| Wrong guesses | 5 per code, then the sign-in starts over |
| New code | at most once a minute, 5 codes per sign-in; each new code replaces the last |
| Emails per address | 8 per hour (sign-in and sign-up together) |
| Storage | SHA-256 of the challenge token; HMAC-SHA256 of the code bound to that challenge |

## API

| Step | Endpoint | Result |
| --- | --- | --- |
| Sign in / sign up | `POST /api/v2/auth/login`, `POST /api/v2/auth/register` | Known computer: tokens. Otherwise **401** `email_verification_required` with `challengeToken`, `challengeExpiresAt` and `emailHint` (masked address). |
| Enter code | `POST /api/v2/auth/email/verify` `{challengeToken, code}` | `AuthTokens` (201 for a sign-up). **422** wrong code; **401** expired, used or out of attempts; **409** the address was registered meanwhile. |
| New code | `POST /api/v2/auth/email/resend` `{challengeToken}` | `{challengeExpiresAt, emailHint}`; **409** too soon or too many codes. |

Older apps that do not know the code step show the server message asking to
update Sanser.

## Server configuration

Email is sent over SMTP. Set these variables on the server (never commit them):

| Variable | Meaning |
| --- | --- |
| `SMTP_HOST` | Mail server, e.g. `smtp.gmail.com`. Turns email codes on. |
| `SMTP_SECURITY` | `starttls` (default, port 587), `tls` (port 465) or `none` (local relay only) |
| `SMTP_PORT` | Optional; defaults from `SMTP_SECURITY` |
| `SMTP_USERNAME`, `SMTP_PASSWORD` | Login for the mail server, set together. For Gmail use an app password, not the account password. |
| `SMTP_FROM` | Sender, e.g. `Sanser <no-reply@example.com>` |

Without `SMTP_HOST` the server logs a warning at startup, signs in with the
password alone and creates accounts without confirming the address. It still
remembers each computer that signs in, so enabling SMTP later does not ask
already-used computers for a code. If sending fails, sign-in and sign-up
answer 503 and nothing is created.
