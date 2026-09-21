ALTER TABLE devices ADD COLUMN device_role TEXT NOT NULL DEFAULT 'unknown'
  CHECK (device_role IN ('host', 'client', 'unknown'));
ALTER TABLE devices ADD COLUMN cross_platform BOOLEAN NOT NULL DEFAULT FALSE;
-- Releases before bidirectional support had Windows hosts and macOS clients only.
UPDATE devices SET device_role = CASE
  WHEN lower(platform) LIKE 'windows%' THEN 'host'
  WHEN lower(platform) LIKE 'mac%' THEN 'client'
  ELSE 'unknown' END;
