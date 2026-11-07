-- Room revisions isolate future voice rounds from previously issued SFU JWTs.
-- They do NOT disconnect an already-connected participant: SFU administration
-- and a live revocation test are still required before production admission.
ALTER TABLE sharing_groups ADD COLUMN voice_revision BIGINT NOT NULL DEFAULT 0
    CHECK (voice_revision >= 0);

CREATE FUNCTION sanser_voice_group_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.state IS DISTINCT FROM NEW.state AND NEW.state = 'stopped'
       OR OLD.host_device_id IS DISTINCT FROM NEW.host_device_id
       OR OLD.user_id IS DISTINCT FROM NEW.user_id THEN
        NEW.voice_revision := OLD.voice_revision + 1;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER voice_group_revision BEFORE UPDATE OF state, host_device_id, user_id
    ON sharing_groups FOR EACH ROW EXECUTE FUNCTION sanser_voice_group_revision();

CREATE FUNCTION sanser_voice_member_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.state = 'accepted' THEN
            UPDATE sharing_groups SET voice_revision = voice_revision + 1
                WHERE id = OLD.sharing_id AND user_id = OLD.user_id;
        END IF;
        RETURN OLD;
    END IF;
    IF OLD.state = 'accepted' AND (
        NEW.state IS DISTINCT FROM OLD.state
        OR NEW.viewer_device_id IS DISTINCT FROM OLD.viewer_device_id
        OR NEW.session_id IS DISTINCT FROM OLD.session_id
        OR NEW.sharing_id IS DISTINCT FROM OLD.sharing_id
        OR NEW.user_id IS DISTINCT FROM OLD.user_id
    ) THEN
        UPDATE sharing_groups SET voice_revision = voice_revision + 1
            WHERE id = OLD.sharing_id AND user_id = OLD.user_id;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER voice_member_revision AFTER UPDATE OF state, viewer_device_id,
    session_id, sharing_id, user_id OR DELETE ON sharing_members
    FOR EACH ROW EXECUTE FUNCTION sanser_voice_member_revision();
