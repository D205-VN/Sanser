-- Groups the separately authenticated host/client roles of one installation.
-- Old rows stay ungrouped until that installation identifies itself again.
ALTER TABLE devices ADD COLUMN computer_id TEXT;
CREATE UNIQUE INDEX devices_computer_role_unique
  ON devices(user_id, computer_id, device_role) WHERE computer_id IS NOT NULL;
