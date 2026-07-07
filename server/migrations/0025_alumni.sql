-- 0025_alumni.sql — Alumni log: retain removed employees' identity.
--
-- Deleting a user hard-deletes the row and cascades their data. To keep a
-- record of former employees (the "Alumni" section), we snapshot their identity
-- into this table just before the delete. No FK on user_id (the user row is
-- gone); removed_by references the acting HR and survives their later removal.

CREATE TABLE IF NOT EXISTS alumni (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id     UUID,                                   -- original users.id (no FK — user deleted)
    name        TEXT NOT NULL,
    email       TEXT NOT NULL,
    role        TEXT NOT NULL,                          -- role at removal
    team_id     UUID,                                   -- team at removal (no FK)
    joined_at   TIMESTAMPTZ,                            -- original users.created_at (tenure start)
    removed_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    removed_by  UUID REFERENCES users (id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_alumni_removed_at ON alumni (removed_at DESC);
