-- The currently shipped native sidecars use Sanser's authenticated direct
-- transport, not the unfinished SNV2 packet format. Keep the database label
-- honest and persist requester readiness so the Windows host never attempts
-- its control connection before the macOS listener is bound.
ALTER TABLE connection_sessions
    DROP CONSTRAINT IF EXISTS connection_sessions_selected_transport_check;

UPDATE connection_sessions
SET selected_transport = 'native'
WHERE selected_transport = 'snv2';

ALTER TABLE connection_sessions
    ADD CONSTRAINT connection_sessions_selected_transport_check
        CHECK (selected_transport IS NULL OR selected_transport IN ('native', 'webrtc')),
    ADD COLUMN requester_ready_at BIGINT;
