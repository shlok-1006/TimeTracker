-- 0049_weekly_hours_employee_notified.sql — Separate "employee was emailed" from the legacy digest stamp.
--
-- Until the weekly-hours change, `weekly_hours_reports.notified_at` was stamped when the HR/PM DIGEST was
-- delivered — the employee was never emailed. The Weekly Report now emails employees on demand, and read
-- `notified_at` as "employee emailed", so every row the old batch had digested showed "Sent" and HR could
-- not email anyone. `employee_notified_at` is the per-employee stamp; `notified_at` keeps its old meaning
-- (digest delivered) as history. Additive and idempotent.

ALTER TABLE weekly_hours_reports
    ADD COLUMN IF NOT EXISTS employee_notified_at TIMESTAMPTZ;   -- NULL until the employee is emailed
