CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS auth_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    device_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    last_seen_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    revoked_at BIGINT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_auth_sessions_user
    ON auth_sessions(user_id, revoked_at, last_seen_at);

CREATE TABLE IF NOT EXISTS access_tokens (
    digest TEXT PRIMARY KEY,
    auth_session_id TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    revoked_at BIGINT,
    FOREIGN KEY (auth_session_id) REFERENCES auth_sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_access_tokens_session
    ON access_tokens(auth_session_id, expires_at);

CREATE TABLE IF NOT EXISTS refresh_tokens (
    digest TEXT PRIMARY KEY,
    auth_session_id TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    revoked_at BIGINT,
    rotated_at BIGINT,
    replaced_by_digest TEXT,
    FOREIGN KEY (auth_session_id) REFERENCES auth_sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_session
    ON refresh_tokens(auth_session_id, expires_at);

CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    platform TEXT NOT NULL,
    os_version TEXT NOT NULL,
    gpu TEXT NOT NULL,
    sanser_version TEXT NOT NULL,
    protocol_version BIGINT NOT NULL,
    online BIGINT NOT NULL DEFAULT 0,
    streaming BIGINT NOT NULL DEFAULT 0,
    pinned BIGINT NOT NULL DEFAULT 0,
    route_address TEXT,
    network_quality TEXT,
    latency_ms BIGINT,
    codecs_json TEXT NOT NULL,
    native_transport BIGINT NOT NULL DEFAULT 0,
    webrtc BIGINT NOT NULL DEFAULT 1,
    audio BIGINT NOT NULL DEFAULT 1,
    gamepad BIGINT NOT NULL DEFAULT 0,
    last_seen_at BIGINT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_devices_user_status
    ON devices(user_id, online, pinned, updated_at);

CREATE TABLE IF NOT EXISTS device_keys (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL,
    public_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    revoked_at BIGINT,
    FOREIGN KEY (device_id) REFERENCES devices(id) ON DELETE CASCADE,
    UNIQUE (device_id, fingerprint)
);

CREATE INDEX IF NOT EXISTS idx_device_keys_device
    ON device_keys(device_id, revoked_at);

CREATE TABLE IF NOT EXISTS connection_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    requester_device_id TEXT NOT NULL,
    host_device_id TEXT NOT NULL,
    state TEXT NOT NULL,
    network_mode TEXT NOT NULL,
    quality_profile TEXT NOT NULL,
    requested_codec TEXT NOT NULL,
    selected_transport TEXT,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    accepted_at BIGINT,
    ended_at BIGINT,
    disconnect_reason TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (requester_device_id) REFERENCES devices(id) ON DELETE CASCADE,
    FOREIGN KEY (host_device_id) REFERENCES devices(id) ON DELETE CASCADE,
    CHECK (state IN ('pending', 'accepted', 'rejected', 'disconnected', 'expired')),
    CHECK (network_mode IN ('auto', 'direct', 'relay')),
    CHECK (quality_profile IN ('auto', 'competitive', 'balanced', 'quality', 'custom')),
    CHECK (requested_codec IN ('auto', 'h264', 'hevc')),
    CHECK (selected_transport IS NULL OR selected_transport IN ('snv2', 'webrtc'))
);

CREATE INDEX IF NOT EXISTS idx_connection_sessions_user_state
    ON connection_sessions(user_id, state, updated_at);
CREATE INDEX IF NOT EXISTS idx_connection_sessions_host_state
    ON connection_sessions(host_device_id, state, updated_at);

CREATE TABLE IF NOT EXISTS connection_events (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    session_id TEXT,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (session_id) REFERENCES connection_sessions(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_connection_events_user_time
    ON connection_events(user_id, created_at, id);

CREATE TABLE IF NOT EXISTS password_reset_tokens (
    digest TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    used_at BIGINT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_expiry
    ON password_reset_tokens(expires_at);

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    action TEXT NOT NULL,
    target_type TEXT,
    target_id TEXT,
    metadata_json TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_user_time
    ON audit_logs(user_id, created_at);

CREATE TABLE IF NOT EXISTS user_preferences (
    user_id TEXT PRIMARY KEY,
    preferences_json TEXT NOT NULL,
    updated_at BIGINT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);
