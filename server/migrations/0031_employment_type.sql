-- Employment type (employee / contractor / intern) — an HR classification of a
-- worker, orthogonal to the RBAC `role`. A contractor or intern still signs in
-- with role='employee'; this only categorises them for HR.
--
-- Note: the enum's `employee` value shares a name with the `user_role` enum's
-- `employee` — harmless, they are distinct Postgres types.
DO $$ BEGIN
    CREATE TYPE employment_type AS ENUM ('employee', 'contractor', 'intern');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS employment_type employment_type NOT NULL DEFAULT 'employee';
