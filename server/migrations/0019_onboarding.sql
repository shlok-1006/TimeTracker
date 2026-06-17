-- 0019_onboarding.sql — Candidate onboarding (Feature 6A).
--
-- A recruiting/onboarding pipeline: candidates move through ordered stages
-- (Kanban columns), carry a checklist of tasks and uploaded documents, and can
-- be converted into an employee user account once hired.

CREATE TABLE IF NOT EXISTS onboarding_stages (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name       TEXT NOT NULL UNIQUE,
    sequence   INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Default pipeline (idempotent).
INSERT INTO onboarding_stages (name, sequence) VALUES
    ('Applied', 1), ('Interview', 2), ('Offer', 3), ('Onboarding', 4), ('Hired', 5)
ON CONFLICT (name) DO NOTHING;

CREATE TABLE IF NOT EXISTS candidates (
    id                UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name              TEXT NOT NULL,
    email             TEXT NOT NULL,
    position          TEXT NOT NULL DEFAULT '',
    stage_id          UUID NOT NULL REFERENCES onboarding_stages (id),
    status            TEXT NOT NULL DEFAULT 'active'
                          CHECK (status IN ('active', 'hired', 'rejected')),
    -- Set once converted to an employee account.
    converted_user_id UUID REFERENCES users (id) ON DELETE SET NULL,
    hired_at          TIMESTAMPTZ,
    created_by        UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_candidates_stage ON candidates (stage_id);

CREATE TABLE IF NOT EXISTS candidate_tasks (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    candidate_id UUID NOT NULL REFERENCES candidates (id) ON DELETE CASCADE,
    title        TEXT NOT NULL,
    done         BOOLEAN NOT NULL DEFAULT FALSE,
    done_at      TIMESTAMPTZ,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_candidate_tasks_candidate ON candidate_tasks (candidate_id);

-- Document METADATA only (Rule 5): bytes live in object storage.
CREATE TABLE IF NOT EXISTS candidate_documents (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    candidate_id UUID NOT NULL REFERENCES candidates (id) ON DELETE CASCADE,
    doc_type     TEXT NOT NULL DEFAULT '',
    storage_key  TEXT NOT NULL UNIQUE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_candidate_documents_candidate ON candidate_documents (candidate_id);
