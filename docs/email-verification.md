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
- **Forgot password** (since 2.3.27): *Forgot password?* on the sign-in
  screen asks for the account email. The server emails a reset code. The
  code is entered and checked first (since 2.3.28); only a correct code
  leads to the new-password step. The code and the new password then set the
  password, sign out **every**
  computer and phone of the account, and sign in the one that asked (it is
  remembered, so it needs no second code). The answer to the request is the
  same whether or not the address has an account: an unknown address gets a
  challenge that sends no email and accepts no code. A reset code never
  completes a normal sign-in, and a sign-in code never resets a password.
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
| Emails per address | 8 per hour (sign-in, sign-up and password reset together) |
| Storage | SHA-256 of the challenge token; HMAC-SHA256 of the code bound to that challenge |

## API

| Step | Endpoint | Result |
| --- | --- | --- |
| Sign in / sign up | `POST /api/v2/auth/login`, `POST /api/v2/auth/register` | Known computer: tokens. Otherwise **401** `email_verification_required` with `challengeToken`, `challengeExpiresAt` and `emailHint` (masked address). |
| Enter code | `POST /api/v2/auth/email/verify` `{challengeToken, code}` | `AuthTokens` (201 for a sign-up). **422** wrong code; **401** expired, used or out of attempts; **409** the address was registered meanwhile. |
| New code | `POST /api/v2/auth/email/resend` `{challengeToken}` | `{challengeExpiresAt, emailHint}`; **409** too soon or too many codes. Also used for reset codes. |
| Forgot password | `POST /api/v2/auth/password/forgot` `{email, deviceName?, platform?, installationId?}` | **202** `{challengeToken, challengeExpiresAt, emailHint}` for any address; **503** when email is not configured. |
| Check reset code | `POST /api/v2/auth/password/check` `{challengeToken, code}` | **204** right code (it stays usable for the next step); **422** wrong code; **401** expired, used or out of attempts. Each check counts as one of the five attempts. |
| Set new password | `POST /api/v2/auth/password/reset` `{challengeToken, code, newPassword}` | `AuthTokens`. **422** wrong code or weak password; **401** expired, used or out of attempts. |

Older apps that do not know the code step show the server message asking to
update Sanser.

## Server configuration

Set these variables on the server (never commit them). Resend is the simplest
choice: it sends over HTTPS, so it also works where outgoing mail ports are
blocked, such as Render's free plan.

**Resend (preferred)**

| Variable | Meaning |
| --- | --- |
| `RESEND_API_KEY` | A Resend API key with sending access. Turns email codes on. |
| `EMAIL_FROM` | Sender on a domain verified in Resend, e.g. `Sanser <no-reply@example.com>`. `SMTP_FROM` is used when it is not set. |

**SMTP (used only when `RESEND_API_KEY` is not set)**

| Variable | Meaning |
| --- | --- |
| `SMTP_HOST` | Mail server, e.g. `smtp.gmail.com`. Turns email codes on. |
| `SMTP_SECURITY` | `starttls` (default, port 587), `tls` (port 465) or `none` (local relay only) |
| `SMTP_PORT` | Optional; defaults from `SMTP_SECURITY` |
| `SMTP_USERNAME`, `SMTP_PASSWORD` | Login for the mail server, set together. For Gmail use an app password, not the account password. |
| `SMTP_FROM` | Sender, e.g. `Sanser <no-reply@example.com>` |

Render's free plan blocks outgoing connections to ports 25, 465 and 587, so
Gmail SMTP times out there. Use Resend, or an SMTP provider that also listens
on port 2525, for example Brevo: `SMTP_HOST=smtp-relay.brevo.com`,
`SMTP_PORT=2525`, `SMTP_SECURITY=starttls`, the Brevo SMTP login and SMTP key
as username and password, and a sender verified in Brevo as `SMTP_FROM`.

Without either service the server logs a warning at startup, signs in with the
password alone and creates accounts without confirming the address. It still
remembers each computer that signs in, so enabling email later does not ask
already-used computers for a code. If sending fails or takes longer than 8
seconds, sign-in and sign-up answer 503 and nothing is created; the reason
(for example an unverified Resend domain) is in the server log.
