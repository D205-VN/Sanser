-- No JWTs/secrets. Missing/revoked login or membership invalidates the mapping,
-- including LiveKit-refreshed tokens which retain only room and identity.
CREATE TABLE voice_lab_admissions (
    room_name TEXT NOT NULL REFERENCES voice_lab_rooms(room_name) ON DELETE CASCADE,
    identity TEXT NOT NULL CHECK (identity ~ '^snp_[A-Za-z0-9_-]{43}$'),
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    auth_session_id TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (room_name, identity)
);
CREATE INDEX voice_lab_admission_login ON voice_lab_admissions(auth_session_id);

-- One source of truth for gateway joins, live checks and closure reconciliation.
-- Clock/freshness bounds are supplied by the server, never by a client.
CREATE FUNCTION sanser_voice_admission_valid(p_room TEXT, p_identity TEXT,
    p_scope TEXT, p_now BIGINT, p_fresh BIGINT, p_idle BIGINT)
RETURNS BOOLEAN LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM voice_lab_admissions a
        JOIN voice_lab_rooms r ON r.room_name=a.room_name AND r.user_id=a.user_id
        JOIN sharing_groups g ON g.id=r.sharing_id AND g.user_id=r.user_id
        JOIN auth_sessions login ON login.id=a.auth_session_id AND login.user_id=a.user_id
        JOIN devices d ON d.id=a.device_id AND d.user_id=a.user_id
        JOIN devices h ON h.id=g.host_device_id AND h.user_id=a.user_id
        WHERE a.room_name=p_room AND a.identity=p_identity AND r.sfu_scope=p_scope
            AND r.retired_at IS NULL AND r.closed_at IS NULL
            AND r.voice_revision=g.voice_revision AND g.state IN ('preparing','active')
            AND login.revoked_at IS NULL AND login.expires_at>p_now
            AND login.last_seen_at BETWEEN p_idle AND p_now
            AND d.online AND d.last_seen_at BETWEEN p_fresh AND p_now
            AND h.online AND h.last_seen_at BETWEEN p_fresh AND p_now
            AND (g.host_device_id=d.id OR EXISTS (
                SELECT 1 FROM sharing_members m JOIN connection_sessions s
                    ON s.id=m.session_id AND s.user_id=m.user_id
                WHERE m.sharing_id=g.id AND m.user_id=g.user_id
                    AND m.viewer_device_id=d.id AND m.state='accepted'
                    AND s.sharing_id=g.id AND s.host_device_id=g.host_device_id
                    AND s.requester_device_id=d.id AND s.state='accepted'
                    AND s.access_mode='view-only' AND s.ended_at IS NULL
            ))
    );
$$;
