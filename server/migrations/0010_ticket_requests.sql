-- 0010_ticket_requests.sql — manual ticket access requests.
-- An employee with no assigned tickets can request access to a ticket by id;
-- the ticket's (parent) owner approves/rejects via an emailed token link.

CREATE TABLE IF NOT EXISTS ticket_requests (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id        UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    ticket_id      TEXT        NOT NULL,
    ticket_title   TEXT,
    owner_email    TEXT,
    status         TEXT        NOT NULL DEFAULT 'pending',
    decision_token TEXT        NOT NULL UNIQUE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at     TIMESTAMPTZ,
    CONSTRAINT ticket_requests_status CHECK (status IN ('pending', 'approved', 'rejected'))
);

CREATE INDEX IF NOT EXISTS idx_ticket_requests_user ON ticket_requests (user_id);
