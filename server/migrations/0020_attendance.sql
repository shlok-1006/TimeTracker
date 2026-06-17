-- 0020_attendance.sql — Attendance derived from intervals (Feature 6C).
--
-- A nightly rollup (and on-demand recompute) derives one row per employee per
-- UTC day from the immutable interval log (Rule 2: totals are always derived,
-- never mutated counters). The derivation integrates approved leave and the
-- company holiday calendar so an absence is explained, not just flagged.
--
--   status:
--     present  — worked >= the full-day threshold
--     partial  — worked some time, below the threshold
--     leave    — no work, covered by an approved leave request
--     holiday  — no work, the day is a company holiday
--     weekend  — no work, Saturday/Sunday
--     absent   — no work and none of the above

CREATE TABLE IF NOT EXISTS attendance_days (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day            DATE NOT NULL,
    status         TEXT NOT NULL
                       CHECK (status IN ('present', 'partial', 'absent', 'leave', 'holiday', 'weekend')),
    worked_seconds INTEGER NOT NULL DEFAULT 0,
    idle_seconds   INTEGER NOT NULL DEFAULT 0,
    -- First clock-in / last clock-out (UTC) among worked intervals that day.
    first_in_utc   TIMESTAMPTZ,
    last_out_utc   TIMESTAMPTZ,
    -- Human-readable context for leave/holiday (e.g. "Annual Leave", "New Year").
    note           TEXT NOT NULL DEFAULT '',
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, day)
);

CREATE INDEX IF NOT EXISTS idx_attendance_user_day ON attendance_days (user_id, day);
CREATE INDEX IF NOT EXISTS idx_attendance_day      ON attendance_days (day);
