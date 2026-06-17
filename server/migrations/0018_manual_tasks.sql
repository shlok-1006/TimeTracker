-- 0018_manual_tasks.sql — HR/PM-assigned tasks (Feature 5).
--
-- A manual task is an internal work item assigned to an employee by HR or a
-- project manager. It is NOT a Linear ticket (it never touches Linear) but is
-- treated like one for AI screenshot analysis: the analyzer compares screenshots
-- against the employee's manual tasks alongside their Linear tickets.

CREATE TABLE IF NOT EXISTS manual_tasks (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    -- The employee the task is assigned to.
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Who created it (HR / project manager). NULL if that account is later removed.
    created_by  UUID REFERENCES users (id) ON DELETE SET NULL,
    title       TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status      TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'done')),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_manual_tasks_user ON manual_tasks (user_id, status);
