-- 0022_weekly_hours_reports.sql — Weekly hours-compliance rollup + warning log.
--
-- Each Mon–Sun week (UTC) an employee is expected to work `working_days × 8h`
-- (a full 5-working-day week = 40h). Working days are business days (Mon–Fri)
-- that were not holidays or approved leave, derived from the attendance rollup.
--
-- One row per (employee, week) records the requirement vs. actual worked time.
-- `notified_at` is stamped once HR and the employee's project manager have been
-- warned about a shortfall, so the Monday-morning job never double-sends.

CREATE TABLE IF NOT EXISTS weekly_hours_reports (
    id                UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id           UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    week_start        DATE    NOT NULL,               -- Monday (UTC)
    week_end          DATE    NOT NULL,               -- Sunday (UTC)
    working_days      INTEGER NOT NULL,
    required_seconds  BIGINT  NOT NULL,
    worked_seconds    BIGINT  NOT NULL,
    shortfall_seconds BIGINT  NOT NULL,
    compliant         BOOLEAN NOT NULL,
    notified_at       TIMESTAMPTZ,                    -- NULL until HR/PM warned
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, week_start)
);

CREATE INDEX IF NOT EXISTS idx_weekly_hours_reports_user ON weekly_hours_reports (user_id);
CREATE INDEX IF NOT EXISTS idx_weekly_hours_reports_week ON weekly_hours_reports (week_start);
