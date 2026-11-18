-- Two-step verification (TOTP, RFC 6238). The shared secret is stored only as
-- an XChaCha20-Poly1305 envelope bound to the account id; recovery codes are
-- stored only as Argon2id hashes.
CREATE TABLE user_two_factor (
    user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    secret_envelope TEXT NOT NULL,
    enabled_at BIGINT,
    -- Highest accepted TOTP step; a code for this step or earlier is refused.
    last_used_step BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE TABLE two_factor_recovery_codes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    used_at BIGINT
);
CREATE INDEX two_factor_recovery_codes_user ON two_factor_recovery_codes(user_id, used_at);

-- Short-lived second-step challenges issued after a correct password.
-- Only a SHA-256 digest of the bearer challenge is stored.
CREATE TABLE two_factor_challenges (
    digest TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    installation_id TEXT,
    attempts INTEGER NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    consumed_at BIGINT
);
CREATE INDEX two_factor_challenges_user ON two_factor_challenges(user_id, created_at);
