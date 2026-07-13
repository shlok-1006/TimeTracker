-- 0028_user_managers.sql — many-to-many employee↔manager assignment.
--
-- An employee can have MULTIPLE project managers, one, or none. This join
-- table replaces the single users.manager_id pointer for every scoping and
-- notification decision (the old column is retained but deprecated — no code
-- reads it after this migration ships). Backfilled from the existing pointers
-- so current assignments carry over unchanged.

CREATE TABLE IF NOT EXISTS user_managers (
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    manager_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    added_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, manager_id),
    CHECK (user_id <> manager_id)
);

CREATE INDEX IF NOT EXISTS idx_user_managers_manager ON user_managers (manager_id);

INSERT INTO user_managers (user_id, manager_id)
SELECT id, manager_id FROM users
WHERE manager_id IS NOT NULL AND manager_id <> id
ON CONFLICT DO NOTHING;
