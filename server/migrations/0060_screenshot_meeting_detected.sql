-- 0060_screenshot_meeting_detected.sql — calls recognised from the screenshot.
--
-- From desktop 0.1.19 there is no manual Meeting mode. The vision analyzer
-- recognises a live Google Meet call or Slack huddle on screen instead; such a
-- shot is re-tagged captured_status = 'meeting' (so it is never sampled, scored
-- or counted as working again) and gets a replacement from the same part of
-- the day.
--
-- meeting_detected_at records that the AI made the change, and when: a
-- 'meeting' row with it NULL was tagged by an older desktop's Meeting mode.

ALTER TABLE screenshots
    ADD COLUMN IF NOT EXISTS meeting_detected_at TIMESTAMPTZ;
