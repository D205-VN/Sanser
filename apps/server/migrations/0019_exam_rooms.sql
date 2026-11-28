-- Exam mode. A supervisor opens a room with a short code; each student joins
-- it from their own account and computer. While the room is open the student
-- computer keeps only the allowed apps in front and reports what it sees.
-- Screenshots never reach the database: they live in server memory briefly.
CREATE TABLE exam_rooms (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    code TEXT NOT NULL,
    -- JSON array of {"id","name"}: macOS bundle identifiers or Windows file names.
    allowed_apps_json TEXT NOT NULL DEFAULT '[]',
    block_ai BOOLEAN NOT NULL DEFAULT TRUE,
    state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'ended')),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    -- An open room ends by itself here, so no student stays locked forever.
    expires_at BIGINT NOT NULL,
    ended_at BIGINT
);
CREATE UNIQUE INDEX exam_rooms_open_code ON exam_rooms(code) WHERE state = 'open';
CREATE INDEX exam_rooms_owner ON exam_rooms(owner_user_id, created_at);

CREATE TABLE exam_members (
    id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL REFERENCES exam_rooms(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    device_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'paused', 'removed')),
    joined_at BIGINT NOT NULL,
    last_seen_at BIGINT NOT NULL,
    foreground_app TEXT,
    violation_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE (room_id, device_id),
    FOREIGN KEY (device_id, user_id) REFERENCES devices(id, user_id) ON DELETE CASCADE
);
CREATE INDEX exam_members_device ON exam_members(user_id, device_id);

CREATE TABLE exam_violations (
    id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL REFERENCES exam_rooms(id) ON DELETE CASCADE,
    member_id TEXT NOT NULL REFERENCES exam_members(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    detail TEXT NOT NULL,
    created_at BIGINT NOT NULL
);
CREATE INDEX exam_violations_room ON exam_violations(room_id, created_at);
