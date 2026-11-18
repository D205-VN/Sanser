-- Guest ("support session") access. A host creates a short-lived grant; a
-- person without an account redeems it once with the access code and the
-- one-time password, then waits for the host to accept.
CREATE TABLE guest_access_grants (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    host_device_id TEXT NOT NULL,
    -- Keyed HMAC of the normalized access code (lookup only, never the code).
    code_digest TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    failed_attempts INTEGER NOT NULL DEFAULT 0,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    redeemed_at BIGINT,
    revoked_at BIGINT,
    session_id TEXT,
    guest_device_id TEXT,
    guest_name TEXT,
    FOREIGN KEY (host_device_id, user_id) REFERENCES devices(id, user_id) ON DELETE CASCADE
);
CREATE INDEX guest_access_grants_host ON guest_access_grants(user_id, host_device_id, created_at);

-- Guest devices live in the host account only so the existing session,
-- signaling and relay checks apply; every account-scoped device query
-- excludes them.
ALTER TABLE devices ADD COLUMN guest_grant_id TEXT;
CREATE INDEX devices_guest_grant ON devices(guest_grant_id) WHERE guest_grant_id IS NOT NULL;

ALTER TABLE connection_sessions ADD COLUMN guest_grant_id TEXT;
ALTER TABLE connection_sessions ADD COLUMN guest_name TEXT;

-- 'account' sessions are ordinary sign-ins. 'guest' sessions carry no refresh
-- token and are limited to one connection session and one guest device.
ALTER TABLE auth_sessions ADD COLUMN scope TEXT NOT NULL DEFAULT 'account'
    CHECK (scope IN ('account', 'guest'));
ALTER TABLE auth_sessions ADD COLUMN guest_grant_id TEXT;
