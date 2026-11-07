-- Only successfully closed rooms are eligible. Pending SFU closure information
-- must survive configuration changes and arbitrarily long upstream outages.
CREATE INDEX voice_lab_closed_retention ON voice_lab_rooms(closed_at, room_name)
    WHERE retired_at IS NOT NULL AND closed_at IS NOT NULL
        AND lease_until=0 AND lease_id IS NULL;
