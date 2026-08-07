-- 0042_employee_directory.sql — the employee record behind the onboarding form.
--
-- Implements the people-side tables from the RUH HRMS "Employee & Teams" schema
-- proposal (6 Aug 2026). That document names three tables we already have under
-- different names, so this migration does NOT recreate them:
--
--   proposal          here              note
--   ---------------   ---------------   ------------------------------------------
--   employees         users             the core person row (0001_init)
--   teams             teams             0016_teams
--   team_members      user_teams        membership (0016_teams); the 'pm' half of
--                     + user_managers   role_in_team is user_managers (0028)
--
-- What IS new is everything the onboarding form collects. The proposal splits it
-- across several tables and the splits ARE the security tiers, so they are kept
-- exactly:
--
--   tier 1 (readable by the app and the chatbot)  users + teams + membership
--   tier 2 (personal — HR/PM scoped)             employee_profiles, _education,
--                                                _prev_employment, _documents
--   tier 3 (sealed — payroll only)               employee_bank_details
--
-- Every table here is keyed on users.id: one home per fact, and adding a person
-- to a second team stays one row in user_teams rather than a schema change.

-- ── Tier 1: the core row ────────────────────────────────────────────────────
-- The proposal's `employees` carries the display code, department, designation
-- and joined date. Those belong with the person's core record (the tier the app
-- and chatbot may read), not with their DOB, so they are columns on `users`.
-- All nullable: existing accounts predate the form and must not be invalidated.
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS employee_code TEXT,
    ADD COLUMN IF NOT EXISTS department    TEXT,
    ADD COLUMN IF NOT EXISTS designation   TEXT,
    ADD COLUMN IF NOT EXISTS joined_on     DATE;

-- Display code (RUH-021). Unique where present; partial index so the many NULLs
-- of not-yet-onboarded accounts don't collide with each other.
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_employee_code
    ON users (employee_code)
    WHERE employee_code IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_users_department ON users (department);

-- ── Tier 2: personal details ────────────────────────────────────────────────
-- One row per person. Separate from `users` because it is a tier up: the app may
-- read the core row freely, but this needs HR/PM scoping.
CREATE TABLE IF NOT EXISTS employee_profiles (
    user_id            UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    date_of_birth      DATE,
    gender             TEXT,
    marital_status     TEXT,
    blood_group        TEXT,
    personal_email     TEXT,
    phone              TEXT,
    current_address    TEXT,
    permanent_address  TEXT,
    emergency_name     TEXT,
    emergency_phone    TEXT,
    emergency_relation TEXT,
    -- Free-form catch-all for form answers that have no column yet. Keeping the
    -- raw response means a question added to the form is never silently dropped
    -- while the schema catches up.
    extra              JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- HR verifies the form's answers; until then the row is the employee's claim.
    verified_at        TIMESTAMPTZ,
    verified_by        UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Qualifications — one row per degree/certificate.
CREATE TABLE IF NOT EXISTS employee_education (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    degree       TEXT NOT NULL,
    institute    TEXT,
    -- Text, not INT: forms collect "2019", "2018-2022" and "Pursuing" alike, and
    -- refusing the answer is worse than storing what the person actually wrote.
    year         TEXT,
    grade        TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_employee_education_user ON employee_education (user_id);

-- Previous companies — one row per past job.
CREATE TABLE IF NOT EXISTS employee_prev_employment (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    company      TEXT NOT NULL,
    title        TEXT,
    from_date    DATE,
    to_date      DATE,
    notes        TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_employee_prev_employment_user ON employee_prev_employment (user_id);

-- Uploaded files from the onboarding form (ID proof, certificates).
-- Mirrors Rule 5: the row holds the storage key, never the bytes.
CREATE TABLE IF NOT EXISTS employee_documents (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    file_name    TEXT,
    storage_key  TEXT NOT NULL,
    uploaded_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_employee_documents_user ON employee_documents (user_id);

-- ── Tier 3: sealed ──────────────────────────────────────────────────────────
-- Its own table on purpose. Payroll access only, and it must never appear in the
-- chatbot's allowed-tables list — being a separate table is what makes "not in
-- the list" a thing you can actually enforce.
CREATE TABLE IF NOT EXISTS employee_bank_details (
    user_id        UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    account_name   TEXT,
    account_number TEXT,
    bank_name      TEXT,
    ifsc           TEXT,
    pan            TEXT,
    uan            TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
