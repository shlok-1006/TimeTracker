-- Monthly summary reports (one row per employee per month).
--
-- A frozen snapshot rather than a live view: HR/PMs generate it on demand at any
-- time, and the scheduler generates it automatically once the month closes, so
-- the numbers a report shows must not silently change afterwards (attendance
-- edits, late analysis runs). Regenerating is an explicit action that upserts.
--
-- `month` is the FIRST DAY of the org-local (IST) month — the same day-key basis
-- as analysis_reports (see server/src/org_time.rs).
CREATE TABLE IF NOT EXISTS monthly_reports (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id            UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    month              DATE NOT NULL,

    -- Time: totals over the month, from attendance_days (worked = active+idle+
    -- meeting, break excluded) plus any manual grace grants that fell in it.
    worked_seconds     BIGINT NOT NULL DEFAULT 0,
    grace_seconds      BIGINT NOT NULL DEFAULT 0,

    -- Attendance day counts.
    days_present       INT NOT NULL DEFAULT 0,
    days_partial       INT NOT NULL DEFAULT 0,
    days_absent        INT NOT NULL DEFAULT 0,
    days_leave         INT NOT NULL DEFAULT 0,
    days_holiday       INT NOT NULL DEFAULT 0,
    days_weekend       INT NOT NULL DEFAULT 0,

    -- Daily-report analysis rolled up over the month.
    days_analyzed      INT NOT NULL DEFAULT 0,   -- days that have an analysis report
    days_above_threshold INT NOT NULL DEFAULT 0, -- days scoring >= alignment_threshold
    alignment_threshold DOUBLE PRECISION NOT NULL DEFAULT 50,
    avg_alignment_score DOUBLE PRECISION,        -- NULL when nothing was analysed
    screenshots_analyzed INT NOT NULL DEFAULT 0,

    -- Per-day series [{day, worked_seconds, status, alignment_score, total_analyzed}]
    -- frozen at generation time so the report renders without recomputation.
    days               JSONB NOT NULL DEFAULT '[]'::jsonb,

    -- NULL = produced by the month-end scheduler; set = the HR/PM who generated it.
    generated_by       UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),

    UNIQUE (user_id, month)
);

-- Roster reads ("everyone's report for month M") and per-user history.
CREATE INDEX IF NOT EXISTS monthly_reports_month_idx ON monthly_reports (month);
CREATE INDEX IF NOT EXISTS monthly_reports_user_month_idx ON monthly_reports (user_id, month DESC);
