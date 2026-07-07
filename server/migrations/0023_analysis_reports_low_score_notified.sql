-- 0023_analysis_reports_low_score_notified.sql — Track HR low-score alerts.
--
-- When a nightly daily report's alignment score falls below the threshold
-- (default 35%), HR is emailed once. This column records when that alert was
-- sent so a re-run of the nightly job (e.g. after a restart) never re-notifies.
-- NULL = not yet alerted. The report upsert never writes this column, so
-- rebuilding a report preserves the stamp.

ALTER TABLE analysis_reports
    ADD COLUMN IF NOT EXISTS low_score_notified_at TIMESTAMPTZ;
