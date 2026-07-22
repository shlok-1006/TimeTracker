-- Reintroduce the "partial" (half-day) attendance status. Migration 0021 had
-- dropped it; this restores it to the CHECK constraint. A completed day with
-- some tracked time but under the full-day threshold (default 4h) is now marked
-- `partial` instead of `present`.
ALTER TABLE attendance_days DROP CONSTRAINT IF EXISTS attendance_days_status_check;
ALTER TABLE attendance_days
    ADD CONSTRAINT attendance_days_status_check
    CHECK (status IN ('present', 'absent', 'leave', 'holiday', 'weekend', 'partial'));
