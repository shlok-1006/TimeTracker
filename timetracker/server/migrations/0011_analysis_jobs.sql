-- 0011_analysis_jobs.sql — Daily Screenshot Sampler (STEP 9).
--
-- One job per (user, day). A job samples 4–5 "Working" screenshots spread
-- across the workday (one per time bucket) for later analysis. Sampling is
-- idempotent: the chosen set is stored in analysis_job_samples and never
-- recomputed for a day that already has a job (UNIQUE(user_id, day)).

CREATE TABLE IF NOT EXISTS analysis_jobs (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day        DATE        NOT NULL,
    -- 'pending' once created, 'sampled' once the set has been chosen.
    status     TEXT        NOT NULL DEFAULT 'pending'
                   CHECK (status IN ('pending', 'sampled')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, day)
);

CREATE INDEX IF NOT EXISTS idx_analysis_jobs_user ON analysis_jobs (user_id, day);

-- The chosen screenshots for a job (the immutable sampled set).
CREATE TABLE IF NOT EXISTS analysis_job_samples (
    job_id        UUID NOT NULL REFERENCES analysis_jobs (id)  ON DELETE CASCADE,
    screenshot_id UUID NOT NULL REFERENCES screenshots (id)    ON DELETE CASCADE,
    -- Which time-of-day bucket this screenshot was drawn from.
    bucket        TEXT NOT NULL,
    PRIMARY KEY (job_id, screenshot_id)
);
