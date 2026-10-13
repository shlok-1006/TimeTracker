-- Who may take each leave type, by employment type (EMP-01).
--
-- HR policy (13 Oct 2026): interns and contractors do not get Earned Leave. Until now that rested on
-- the per-category defaults alone (0032: default_days_contractor / default_days_intern = 0), which an
-- HR allocation for the year overrides outright. This makes it a rule of the type: a person whose
-- category is not in `employment_types` gets 0 days of it, cannot request it, and cannot be allocated it.
--
-- The category is the same one the defaults use: project managers and HR count as employees, whatever
-- their employment_type; everyone else is their employment_type.
--
-- Every existing type keeps everyone (the default); Earned Leave becomes employees only.
ALTER TABLE leave_types
    ADD COLUMN IF NOT EXISTS employment_types employment_type[] NOT NULL
        DEFAULT ARRAY['employee', 'contractor', 'intern']::employment_type[];

ALTER TABLE leave_types
    ADD CONSTRAINT leave_types_employment_types_not_empty
        CHECK (cardinality(employment_types) >= 1);

COMMENT ON COLUMN leave_types.employment_types IS
    'Employment types (PM/HR count as employee) that may take this leave. Others get 0 days, cannot '
    'request it, and cannot be allocated it.';

UPDATE leave_types
   SET employment_types = ARRAY['employee']::employment_type[]
 WHERE lower(btrim(name)) IN ('earned leave', 'earned leaves');
