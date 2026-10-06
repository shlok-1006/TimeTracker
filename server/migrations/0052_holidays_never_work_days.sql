-- 0052_holidays_never_work_days.sql — company holidays are never a work day, like weekends.
--
-- `attendance_service::derive_status` now keeps a holiday as `holiday` even when the employee tracked
-- time on it (before, ANY tracked minute turned it into `present`/`partial`). Because the Weekly Report
-- counts present/partial/absent weekdays as required working days, a worked holiday added 8h to that
-- week's requirement — e.g. a Friday holiday made the week 40h instead of 32h.
--
-- The code change only affects days rolled up from now on. This one-off correction repairs history:
--
--   1. holiday days saved as `present`/`partial` become `holiday` (named), keeping their worked/idle
--      seconds — the hours still count, exactly as weekend hours do. HR overrides are left untouched,
--      as 0039 did for weekends;
--   2. each weekly_hours_reports row whose week contained a day fixed in step 1 loses exactly those
--      days from `working_days`, and its requirement / shortfall / compliance are recomputed with the
--      same formula as weekly_hours_service::evaluate (working_days × 8h). `worked_seconds` and the
--      "employee emailed" stamp are kept. Weeks without a fixed day are not touched.
--
-- One statement (a data-modifying CTE), so step 2 sees exactly the rows step 1 changed. Numbered 0052:
-- 0051 is taken on the unmerged feat/leave-half-period branch.

WITH fixed AS (
    UPDATE attendance_days ad
       SET status = 'holiday', note = h.name, updated_at = now()
      FROM holidays h
     WHERE ad.day = h.day
       AND ad.status IN ('present', 'partial')
       AND ad.is_override = FALSE
    RETURNING ad.user_id, ad.day
),
per_week AS (
    SELECT wh.id, COUNT(*)::int AS fixed_days
      FROM fixed f
      JOIN weekly_hours_reports wh
        ON wh.user_id = f.user_id
       AND f.day BETWEEN wh.week_start AND wh.week_end
     WHERE EXTRACT(ISODOW FROM f.day) < 6          -- only Mon–Fri ever counted as working days
     GROUP BY wh.id
)
UPDATE weekly_hours_reports wh
   SET working_days      = GREATEST(wh.working_days - pw.fixed_days, 0),
       required_seconds  = GREATEST(wh.working_days - pw.fixed_days, 0)::bigint * 8 * 3600,
       shortfall_seconds = GREATEST(GREATEST(wh.working_days - pw.fixed_days, 0)::bigint * 8 * 3600
                                    - wh.worked_seconds, 0),
       compliant         = wh.worked_seconds >= GREATEST(wh.working_days - pw.fixed_days, 0)::bigint * 8 * 3600,
       updated_at        = now()
  FROM per_week pw
 WHERE wh.id = pw.id;
