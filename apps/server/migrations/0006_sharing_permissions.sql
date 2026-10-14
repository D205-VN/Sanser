ALTER TABLE connection_sessions ADD COLUMN access_mode TEXT NOT NULL DEFAULT 'control'
    CHECK (access_mode IN ('control', 'view-only'));

-- Tenant ownership is enforced in SQL as well as in the API.
CREATE UNIQUE INDEX devices_id_user_unique ON devices(id, user_id);
CREATE TABLE sharing_groups (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    host_device_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'preparing' CHECK (state IN ('preparing', 'stopped')),
    max_viewers SMALLINT NOT NULL DEFAULT 3 CHECK (max_viewers BETWEEN 1 AND 3),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    UNIQUE (id, user_id),
    FOREIGN KEY (host_device_id, user_id) REFERENCES devices(id, user_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX sharing_one_open_group_per_host ON sharing_groups(host_device_id)
    WHERE state = 'preparing';

CREATE TABLE sharing_members (
    id TEXT PRIMARY KEY,
    sharing_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    viewer_device_id TEXT NOT NULL,
    slot SMALLINT NOT NULL CHECK (slot BETWEEN 1 AND 3),
    state TEXT NOT NULL DEFAULT 'invited'
        CHECK (state IN ('invited', 'accepted', 'declined', 'removed')),
    access_mode TEXT NOT NULL DEFAULT 'view-only' CHECK (access_mode = 'view-only'),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    FOREIGN KEY (sharing_id, user_id) REFERENCES sharing_groups(id, user_id) ON DELETE CASCADE,
    FOREIGN KEY (viewer_device_id, user_id) REFERENCES devices(id, user_id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX sharing_active_slot ON sharing_members(sharing_id, slot)
    WHERE state IN ('invited', 'accepted');
CREATE UNIQUE INDEX sharing_one_reservation_per_viewer ON sharing_members(viewer_device_id)
    WHERE state IN ('invited', 'accepted');
CREATE INDEX sharing_groups_account ON sharing_groups(user_id, updated_at);
