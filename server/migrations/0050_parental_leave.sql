-- 0050 · Paternity and maternity leave.
--
-- Leave types gain an ELIGIBILITY rule and a DAY BASIS:
--   eligible_gender     NULL = everyone; 'male' / 'female' = only people whose profile gender matches
--                       (employee_profiles.gender, free text from the onboarding form — matched
--                       case-insensitively: "Male"/"M" → male, "Female"/"F" → female)
--   min_tenure_months   0 = from day one; N = only once users.joined_on + N months ≤ the leave's start date
--                       (no joining date on record → not eligible; HR can still grant it as an allocation)
--   day_basis           'working' = weekdays minus holidays (every existing type);
--                       'calendar' = every day in the range (maternity is a continuous 26-week absence —
--                       180 working days would be ~36 weeks)
-- An HR allocation (leave_allocations) still overrides the rule, so HR can grant a type to anyone.
--
-- Policy (as asked): Paternity 10 working days for male employees after 1 year; Maternity 180 calendar days
-- for female employees after 1 year; the same for every employment category.
--
-- Existing types are matched case-insensitively by name ("Maternity leave", "maternity", …) and brought
-- to this policy IN PLACE — so production never ends up with a second, rule-free maternity type that anyone
-- could book. A type is only inserted when none exists.

ALTER TABLE leave_types
    ADD COLUMN IF NOT EXISTS eligible_gender   TEXT CHECK (eligible_gender IN ('male', 'female')),
    ADD COLUMN IF NOT EXISTS min_tenure_months INTEGER NOT NULL DEFAULT 0 CHECK (min_tenure_months >= 0),
    ADD COLUMN IF NOT EXISTS day_basis         TEXT NOT NULL DEFAULT 'working'
                                               CHECK (day_basis IN ('working', 'calendar'));

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM leave_types WHERE lower(name) LIKE '%paternity%') THEN
        UPDATE leave_types
           SET paid = TRUE, default_days = 10, default_days_contractor = 10, default_days_intern = 10,
               eligible_gender = 'male', min_tenure_months = 12, day_basis = 'working'
         WHERE lower(name) LIKE '%paternity%';
    ELSE
        INSERT INTO leave_types (name, paid, default_days, default_days_contractor, default_days_intern,
                                 eligible_gender, min_tenure_months, day_basis)
        VALUES ('Paternity Leave', TRUE, 10, 10, 10, 'male', 12, 'working');
    END IF;

    IF EXISTS (SELECT 1 FROM leave_types WHERE lower(name) LIKE '%maternity%') THEN
        UPDATE leave_types
           SET paid = TRUE, default_days = 180, default_days_contractor = 180, default_days_intern = 180,
               eligible_gender = 'female', min_tenure_months = 12, day_basis = 'calendar'
         WHERE lower(name) LIKE '%maternity%';
    ELSE
        INSERT INTO leave_types (name, paid, default_days, default_days_contractor, default_days_intern,
                                 eligible_gender, min_tenure_months, day_basis)
        VALUES ('Maternity Leave', TRUE, 180, 180, 180, 'female', 12, 'calendar');
    END IF;
END $$;
