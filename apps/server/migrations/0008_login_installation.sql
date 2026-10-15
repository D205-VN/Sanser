-- Older anonymous sessions cannot safely be attributed to a particular computer.
-- Preserve them; only new clients provide an installation ID after password verification.
ALTER TABLE auth_sessions ADD COLUMN installation_id TEXT;
CREATE UNIQUE INDEX idx_auth_sessions_active_installation
    ON auth_sessions (user_id, installation_id)
    WHERE installation_id IS NOT NULL AND revoked_at IS NULL;
