-- Wake-on-LAN: opt-in, host-reported LAN interfaces (MAC + IPv4 broadcast).
-- NULL means the host has not opted in or has withdrawn the information.
ALTER TABLE devices ADD COLUMN wake_on_lan_json TEXT;

-- Connection history reuses connection_sessions. The server records when a
-- session's media first used its relay so the access log can show the route.
ALTER TABLE connection_sessions ADD COLUMN relay_used_at BIGINT;

CREATE INDEX IF NOT EXISTS connection_sessions_history
    ON connection_sessions(user_id, created_at DESC, id DESC);
