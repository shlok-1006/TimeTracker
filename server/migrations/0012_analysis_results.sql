-- 0012_analysis_results.sql — Vision AI Screenshot Analyzer (STEP 10).
--
-- One validated analysis per sampled screenshot: the model's verdict on whether
-- the screen matches the employee's assigned Linear tickets. Bytes are never
-- stored (Rule 5) — only the structured judgement.

CREATE TABLE IF NOT EXISTS analysis_results (
    id                  UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    job_id              UUID NOT NULL REFERENCES analysis_jobs (id) ON DELETE CASCADE,
    screenshot_id       UUID NOT NULL REFERENCES screenshots (id)  ON DELETE CASCADE,
    -- Constrained to the four valid verdicts.
    verdict             TEXT NOT NULL
                            CHECK (verdict IN ('aligned', 'partially_aligned', 'not_aligned', 'inconclusive')),
    -- Best-matching ticket id (Linear), or NULL when none / not applicable.
    matched_ticket      TEXT,
    confidence          DOUBLE PRECISION NOT NULL,
    observed            TEXT NOT NULL,
    rationale           TEXT NOT NULL,
    inconclusive_reason TEXT,
    model               TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- One result per screenshot per job (re-analysis upserts).
    UNIQUE (job_id, screenshot_id)
);

CREATE INDEX IF NOT EXISTS idx_analysis_results_job ON analysis_results (job_id);
