-- 0015_analysis_reports.sql — daily AI work-verification report (Feature 1).
--
-- One row per (user, day): an aggregate of that day's per-screenshot verdicts
-- (analysis_results) plus an alignment score and an optional natural-language
-- summary. Built from a completed analysis job; never stores screenshot bytes.

CREATE TABLE IF NOT EXISTS analysis_reports (
    id                 UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id            UUID NOT NULL REFERENCES users (id)          ON DELETE CASCADE,
    day                DATE NOT NULL,
    job_id             UUID NOT NULL REFERENCES analysis_jobs (id)  ON DELETE CASCADE,
    total_analyzed     INTEGER NOT NULL DEFAULT 0,
    aligned_count      INTEGER NOT NULL DEFAULT 0,
    partially_count    INTEGER NOT NULL DEFAULT 0,
    not_aligned_count  INTEGER NOT NULL DEFAULT 0,
    inconclusive_count INTEGER NOT NULL DEFAULT 0,
    -- 0–100 weighted alignment (aligned=1, partial=0.5; inconclusive excluded).
    alignment_score    DOUBLE PRECISION NOT NULL DEFAULT 0,
    summary_text       TEXT NOT NULL DEFAULT '',
    model              TEXT NOT NULL DEFAULT '',
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, day)
);

CREATE INDEX IF NOT EXISTS idx_analysis_reports_user ON analysis_reports (user_id, day);
