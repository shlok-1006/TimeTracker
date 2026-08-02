-- Weekends never count as a work day: a Saturday/Sunday is always `weekend`,
-- no matter how much was tracked (matches `attendance_service::derive_status`,
-- which now resolves weekends before the tracked-time present/partial branch).
--
-- The derivation change only affects future/recomputed rows; this one-off
-- correction fixes weekend days already saved as `present`/`partial` before the
-- rule existed. HR manual overrides (`is_override = TRUE`) are left untouched.
-- `day` is the UTC calendar day, so its day-of-week is computed directly
-- (0 = Sunday, 6 = Saturday), consistent with how the status is derived.
UPDATE attendance_days
SET status = 'weekend', note = '', updated_at = now()
WHERE status IN ('present', 'partial')
  AND is_override = FALSE
  AND EXTRACT(DOW FROM day) IN (0, 6);
