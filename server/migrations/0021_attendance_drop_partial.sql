-- 0021_attendance_drop_partial.sql — Remove the "partial" attendance tier.
--
-- Any worked time now counts as a full "present" day (a started timer = present).
-- Reclassify existing partial rows and tighten the status CHECK accordingly.

UPDATE attendance_days SET status = 'present' WHERE status = 'partial';

ALTER TABLE attendance_days DROP CONSTRAINT IF EXISTS attendance_days_status_check;
ALTER TABLE attendance_days ADD CONSTRAINT attendance_days_status_check
    CHECK (status IN ('present', 'absent', 'leave', 'holiday', 'weekend'));
