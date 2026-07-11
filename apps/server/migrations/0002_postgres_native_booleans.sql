-- Sanser 2 is PostgreSQL/Neon-only. Convert the compatibility-era integer
-- flags into native PostgreSQL booleans so their types match the API model.
ALTER TABLE devices
    ALTER COLUMN online DROP DEFAULT,
    ALTER COLUMN streaming DROP DEFAULT,
    ALTER COLUMN pinned DROP DEFAULT,
    ALTER COLUMN native_transport DROP DEFAULT,
    ALTER COLUMN webrtc DROP DEFAULT,
    ALTER COLUMN audio DROP DEFAULT,
    ALTER COLUMN gamepad DROP DEFAULT;

ALTER TABLE devices
    ALTER COLUMN online TYPE BOOLEAN USING (online <> 0),
    ALTER COLUMN streaming TYPE BOOLEAN USING (streaming <> 0),
    ALTER COLUMN pinned TYPE BOOLEAN USING (pinned <> 0),
    ALTER COLUMN native_transport TYPE BOOLEAN USING (native_transport <> 0),
    ALTER COLUMN webrtc TYPE BOOLEAN USING (webrtc <> 0),
    ALTER COLUMN audio TYPE BOOLEAN USING (audio <> 0),
    ALTER COLUMN gamepad TYPE BOOLEAN USING (gamepad <> 0);

ALTER TABLE devices
    ALTER COLUMN online SET DEFAULT FALSE,
    ALTER COLUMN streaming SET DEFAULT FALSE,
    ALTER COLUMN pinned SET DEFAULT FALSE,
    ALTER COLUMN native_transport SET DEFAULT FALSE,
    ALTER COLUMN webrtc SET DEFAULT TRUE,
    ALTER COLUMN audio SET DEFAULT TRUE,
    ALTER COLUMN gamepad SET DEFAULT FALSE;

