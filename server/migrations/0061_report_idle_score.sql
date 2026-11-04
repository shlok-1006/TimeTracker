-- A day's score takes idle time into account (HR, 4 Nov 2026).
--
-- `alignment_score` stays THE day score — the Daily Report, the low-score alert and the monthly average all read it —
-- but it is now the screenshot alignment scaled down by idle beyond a 30-minute allowance:
--
--     score = screen_alignment × worked / (worked + max(0, idle − 30 min))
--
-- These columns keep what it was made from, so the screenshot-only alignment can be shown beside it:
--   screen_alignment_score  the verdicts alone (what alignment_score used to be)
--   worked_seconds          active + meeting time that day (attendance's rule)
--   idle_seconds            idle time that day
-- All NULL on reports built before this change: their alignment_score is still screenshot-only.

ALTER TABLE analysis_reports
    ADD COLUMN IF NOT EXISTS screen_alignment_score DOUBLE PRECISION,
    ADD COLUMN IF NOT EXISTS worked_seconds         INTEGER,
    ADD COLUMN IF NOT EXISTS idle_seconds           INTEGER;
