-- Email sign-in verification replaces two-step verification (TOTP).
-- A computer that has signed in to an account before is trusted for that
-- account; any other computer must enter a 6-digit code sent to the account
-- email. Sign-up proves the email address the same way before the account
-- exists. See docs/email-verification.md.
DROP TABLE IF EXISTS two_factor_challenges;
DROP TABLE IF EXISTS two_factor_recovery_codes;
DROP TABLE IF EXISTS user_two_factor;

ALTER TABLE users ADD COLUMN email_verified_at BIGINT;

-- Installations (computers or phones) that completed a sign-in to the account.
-- The installation ID is a random per-installation UUID that the server never
-- returns to any client.
CREATE TABLE trusted_devices (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    installation_id TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    last_used_at BIGINT NOT NULL,
    PRIMARY KEY (user_id, installation_id)
);

-- Computers that already signed in keep working without an email.
INSERT INTO trusted_devices (user_id, installation_id, created_at, last_used_at)
SELECT user_id, installation_id, MIN(created_at), MAX(last_seen_at)
FROM auth_sessions
WHERE installation_id IS NOT NULL AND scope = 'account'
GROUP BY user_id, installation_id
ON CONFLICT DO NOTHING;

-- Pending sign-in or sign-up codes. Only a SHA-256 digest of the bearer
-- challenge and a keyed HMAC of the code are stored. A sign-up challenge
-- carries the new account's password hash; the user row is created only after
-- the code is entered, so nobody can hold an email address without owning it.
CREATE TABLE email_challenges (
    digest TEXT PRIMARY KEY,
    purpose TEXT NOT NULL CHECK (purpose IN ('login', 'register')),
    user_id TEXT REFERENCES users(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    display_name TEXT,
    password_hash TEXT,
    code_mac TEXT NOT NULL,
    device_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    installation_id TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    sends INTEGER NOT NULL DEFAULT 1,
    created_at BIGINT NOT NULL,
    last_sent_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    consumed_at BIGINT,
    CHECK ((purpose = 'login') = (user_id IS NOT NULL)),
    CHECK ((purpose = 'register') = (password_hash IS NOT NULL))
);
CREATE INDEX email_challenges_email ON email_challenges(email, created_at);
