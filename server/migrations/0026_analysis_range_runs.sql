-- On-demand range analysis runs (admin "analyze every screenshot in a range").
-- One row per triggered run; the background task updates the counters so the
-- admin UI can poll progress. Unlike analysis_jobs (one per user+day, sampled),
-- a range run covers an arbitrary [from, to) window and every working shot in it.

CREATE TABLE analysis_range_runs (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    requested_by  UUID REFERENCES users(id) ON DELETE SET NULL,
    from_utc      TIMESTAMPTZ NOT NULL,
    to_utc        TIMESTAMPTZ NOT NULL,
    status        TEXT NOT NULL DEFAULT 'running'
                  CHECK (status IN ('running', 'completed', 'failed')),
    total         INTEGER NOT NULL DEFAULT 0,
    analyzed      INTEGER NOT NULL DEFAULT 0,
    skipped       INTEGER NOT NULL DEFAULT 0,
    failed        INTEGER NOT NULL DEFAULT 0,
    error         TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at   TIMESTAMPTZ
);

CREATE INDEX idx_analysis_range_runs_user ON analysis_range_runs (user_id, created_at DESC);
