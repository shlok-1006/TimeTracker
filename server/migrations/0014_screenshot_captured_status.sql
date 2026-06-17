-- 0014_screenshot_captured_status.sql — Feature 2 Phase 1.
--
-- Tag each screenshot with the employee's presence status at capture time so
-- downstream sampling/analysis can exclude non-working (e.g. meeting) shots.
-- Existing rows default to 'working': historically the desktop captured only
-- while Working, so the backfill is accurate.

ALTER TABLE screenshots
    ADD COLUMN IF NOT EXISTS captured_status TEXT NOT NULL DEFAULT 'working'
        CHECK (captured_status IN ('working', 'meeting', 'break', 'idle', 'not_working'));

CREATE INDEX IF NOT EXISTS idx_screenshots_status
    ON screenshots (user_id, captured_status, taken_at);
