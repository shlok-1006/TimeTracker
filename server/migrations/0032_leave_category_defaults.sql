-- Per-employment-type default leave allotments. The existing `default_days`
-- column is the default for regular employees (and, by policy, project managers
-- and HR, who are treated as the employee category). Contractors and interns get
-- their own defaults. A user's effective allotment for a leave type falls back to
-- their category's default when there is no explicit per-user allocation override.
ALTER TABLE leave_types
    ADD COLUMN default_days_contractor DOUBLE PRECISION NOT NULL DEFAULT 0,
    ADD COLUMN default_days_intern     DOUBLE PRECISION NOT NULL DEFAULT 0;
