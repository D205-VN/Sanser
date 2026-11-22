-- "Forgot password": a 6-digit code emailed to the account address lets the
-- person set a new password. It reuses email_challenges with purpose 'reset'.
-- A request for an address without an account stores a decoy challenge
-- (user_id NULL) that never sends mail and never accepts a code, so the
-- response does not reveal whether the address has an account.
-- See docs/email-verification.md.
DO $$
DECLARE constraint_name text;
BEGIN
    FOR constraint_name IN
        SELECT conname FROM pg_constraint
        WHERE conrelid = 'email_challenges'::regclass AND contype = 'c'
    LOOP
        EXECUTE format('ALTER TABLE email_challenges DROP CONSTRAINT %I', constraint_name);
    END LOOP;
END $$;

ALTER TABLE email_challenges
    ADD CONSTRAINT email_challenges_purpose_valid
        CHECK (purpose IN ('login', 'register', 'reset')),
    ADD CONSTRAINT email_challenges_login_has_user
        CHECK (purpose <> 'login' OR user_id IS NOT NULL),
    ADD CONSTRAINT email_challenges_register_has_no_user
        CHECK (purpose <> 'register' OR user_id IS NULL),
    ADD CONSTRAINT email_challenges_password_only_for_register
        CHECK ((purpose = 'register') = (password_hash IS NOT NULL));

