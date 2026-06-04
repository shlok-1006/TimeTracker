-- 0001_init.sql — foundational schema.
--
-- Conventions:
--   * All timestamps are TIMESTAMPTZ stored in UTC (Rule 3).
--   * Roles are a Postgres enum, never magic strings (CLAUDE.md / Role enum).
--   * Audit logs are append-only (Audit Logging section).

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Allowed roles: employee | project_manager | hr.
--   employee        -> desktop app user
--   project_manager -> admin dashboard user (team management)
--   hr              -> admin dashboard user (full people/admin access)
DO $$
BEGIN
    CREATE TYPE user_role AS ENUM ('employee', 'project_manager', 'hr');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END
$$;

CREATE TABLE IF NOT EXISTS users (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name          TEXT        NOT NULL,
    email         TEXT        NOT NULL UNIQUE,
    password_hash TEXT        NOT NULL,
    role          user_role   NOT NULL DEFAULT 'employee',
    -- Manager who owns this user (RBAC: managers see only their team).
    manager_id    UUID        REFERENCES users (id) ON DELETE SET NULL,
    team_id       UUID,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_users_email      ON users (email);
CREATE INDEX IF NOT EXISTS idx_users_role       ON users (role);
CREATE INDEX IF NOT EXISTS idx_users_manager_id ON users (manager_id);
CREATE INDEX IF NOT EXISTS idx_users_team_id    ON users (team_id);

-- Append-only audit trail. Only admins may read it (enforced in the API layer).
CREATE TABLE IF NOT EXISTS audit_logs (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    actor_id    UUID        REFERENCES users (id) ON DELETE SET NULL,
    action      TEXT        NOT NULL,
    entity_type TEXT        NOT NULL,
    entity_id   UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_actor_id   ON audit_logs (actor_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at ON audit_logs (created_at);

-- Enforce immutability of audit logs: block UPDATE and DELETE.
CREATE OR REPLACE FUNCTION reject_audit_mutation() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'audit_logs are immutable';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS audit_logs_immutable ON audit_logs;
CREATE TRIGGER audit_logs_immutable
    BEFORE UPDATE OR DELETE ON audit_logs
    FOR EACH ROW EXECUTE FUNCTION reject_audit_mutation();
