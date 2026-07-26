-- Manually-granted "grace" time added to an employee's weekly total by HR or a
-- project manager, with a required reason. Kept separate from the immutable
-- interval log (Rule 2 — never mutate/inject worked time): the displayed week
-- total is intervals + grace, tagged when grace > 0. `week_start` is the Monday
-- of the granted business week (4 AM boundary, in the employee's timezone),
-- matching how `hours_summary` scopes "this week".
CREATE TABLE IF NOT EXISTS time_grants (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    week_start DATE NOT NULL,
    seconds    INTEGER NOT NULL CHECK (seconds > 0),
    reason     TEXT NOT NULL,
    granted_by UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_time_grants_user_week ON time_grants (user_id, week_start);
