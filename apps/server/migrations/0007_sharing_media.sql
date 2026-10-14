-- Link each consented viewer to its own authenticated view-only media session.
ALTER TABLE sharing_groups DROP CONSTRAINT sharing_groups_state_check;
ALTER TABLE sharing_groups ADD CONSTRAINT sharing_groups_state_check
    CHECK (state IN ('preparing', 'active', 'stopped'));
DROP INDEX sharing_one_open_group_per_host;
CREATE UNIQUE INDEX sharing_one_open_group_per_host ON sharing_groups(host_device_id)
    WHERE state IN ('preparing', 'active');
ALTER TABLE sharing_groups ADD COLUMN network_mode TEXT NOT NULL DEFAULT 'auto'
    CHECK (network_mode IN ('auto', 'direct', 'relay'));
ALTER TABLE sharing_groups ADD COLUMN quality_profile TEXT NOT NULL DEFAULT 'balanced'
    CHECK (quality_profile IN ('auto', 'competitive', 'balanced', 'quality'));

ALTER TABLE connection_sessions ADD COLUMN sharing_id TEXT;
ALTER TABLE connection_sessions ADD CONSTRAINT session_sharing_owner
    FOREIGN KEY (sharing_id, user_id) REFERENCES sharing_groups(id, user_id) ON DELETE CASCADE;
CREATE UNIQUE INDEX connection_sessions_id_user_unique ON connection_sessions(id, user_id);
ALTER TABLE sharing_members ADD COLUMN session_id TEXT;
ALTER TABLE sharing_members ADD COLUMN started_at BIGINT;
ALTER TABLE sharing_members ADD CONSTRAINT member_session_owner
    FOREIGN KEY (session_id, user_id) REFERENCES connection_sessions(id, user_id)
    ON DELETE SET NULL (session_id);
CREATE UNIQUE INDEX sharing_session_unique ON sharing_members(session_id) WHERE session_id IS NOT NULL;
CREATE INDEX connection_sessions_sharing ON connection_sessions(sharing_id, state);
