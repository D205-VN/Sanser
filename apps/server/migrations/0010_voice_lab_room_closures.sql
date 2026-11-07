-- Durable registry/outbox for rooms actually issued by this Sanser backend.
-- Deliberately no FK to sharing_groups/users: deleting them must not erase the
-- information needed to close a room at the SFU. No JWT/API secret is stored.
CREATE TABLE voice_lab_rooms (
    room_name TEXT PRIMARY KEY CHECK (room_name ~ '^snvl_[A-Za-z0-9_-]{43}$'),
    sfu_scope TEXT NOT NULL CHECK (sfu_scope ~ '^[A-Za-z0-9_-]{43}$'),
    sharing_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    voice_revision BIGINT NOT NULL CHECK (voice_revision >= 0),
    created_at BIGINT NOT NULL,
    retired_at BIGINT,
    closed_at BIGINT,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at BIGINT NOT NULL DEFAULT 0,
    lease_until BIGINT NOT NULL DEFAULT 0,
    lease_id TEXT,
    UNIQUE (sfu_scope, sharing_id, voice_revision)
);
CREATE INDEX voice_lab_pending_closures ON voice_lab_rooms(sfu_scope, next_attempt_at)
    WHERE retired_at IS NOT NULL AND closed_at IS NULL;

CREATE FUNCTION sanser_voice_retire_rooms() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        UPDATE voice_lab_rooms SET retired_at=COALESCE(retired_at,EXTRACT(EPOCH FROM clock_timestamp())::BIGINT)
            WHERE sharing_id=OLD.id AND user_id=OLD.user_id AND retired_at IS NULL;
        RETURN OLD;
    END IF;
    IF NEW.voice_revision IS DISTINCT FROM OLD.voice_revision OR NEW.state='stopped' THEN
        UPDATE voice_lab_rooms SET retired_at=COALESCE(retired_at,EXTRACT(EPOCH FROM clock_timestamp())::BIGINT)
            WHERE sharing_id=OLD.id AND user_id=OLD.user_id AND voice_revision < NEW.voice_revision AND retired_at IS NULL;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER voice_retire_rooms AFTER UPDATE OF voice_revision, state OR DELETE
    ON sharing_groups FOR EACH ROW EXECUTE FUNCTION sanser_voice_retire_rooms();
