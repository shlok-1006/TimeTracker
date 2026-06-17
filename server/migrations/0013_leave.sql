-- 0013_leave.sql — Leave management (HR module).
--
-- Leave types with per-employee yearly allocations, leave requests with an
-- approval workflow, and a company holiday calendar. Day counts use DOUBLE
-- PRECISION (supports half-days) to avoid a NUMERIC/bigdecimal dependency.

CREATE TABLE IF NOT EXISTS leave_types (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name         TEXT    NOT NULL UNIQUE,
    paid         BOOLEAN NOT NULL DEFAULT TRUE,
    default_days DOUBLE PRECISION NOT NULL DEFAULT 0,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS holidays (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    day        DATE NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Per-employee, per-type, per-year entitlement.
CREATE TABLE IF NOT EXISTS leave_allocations (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id       UUID NOT NULL REFERENCES users (id)       ON DELETE CASCADE,
    leave_type_id UUID NOT NULL REFERENCES leave_types (id) ON DELETE CASCADE,
    year          INTEGER NOT NULL,
    allotted_days DOUBLE PRECISION NOT NULL DEFAULT 0,
    UNIQUE (user_id, leave_type_id, year)
);

CREATE TABLE IF NOT EXISTS leave_requests (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id       UUID NOT NULL REFERENCES users (id)       ON DELETE CASCADE,
    leave_type_id UUID NOT NULL REFERENCES leave_types (id) ON DELETE RESTRICT,
    start_date    DATE NOT NULL,
    end_date      DATE NOT NULL,
    days          DOUBLE PRECISION NOT NULL,
    reason        TEXT NOT NULL DEFAULT '',
    status        TEXT NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'approved', 'rejected', 'cancelled')),
    approver_id   UUID REFERENCES users (id),
    decided_at    TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (end_date >= start_date)
);

CREATE INDEX IF NOT EXISTS idx_leave_requests_user   ON leave_requests (user_id, status);
CREATE INDEX IF NOT EXISTS idx_leave_requests_status ON leave_requests (status);
