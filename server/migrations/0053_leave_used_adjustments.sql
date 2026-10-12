-- 0053_leave_used_adjustments.sql — HR can set how many days of a leave type someone has USED.
--
-- "Used" is not stored: it is the sum of the person's approved leave requests for that type and year
-- (db::leave::balances_as_of, remaining_paid_by_user). That leaves no way to record leave taken outside
-- TimeTracker — before it was rolled out, or booked another way — so HR could correct the allotment but not
-- the used figure.
--
-- One row per (user, type, year) holds the days HR recorded ON TOP of the approved requests:
--
--     used = SUM(approved requests) + days          remaining = allotted − used
--
-- HR edits the total; the server stores the difference (POST /admin/leave/used). It is never negative: HR
-- can't set "used" below what approved requests already account for — those are cancelled or rejected
-- instead, so every recorded leave stays explainable — nor above the allotment. A row whose difference
-- becomes 0 is deleted. Saves take the same per-person lock as leave approval, so the two can't race.

CREATE TABLE IF NOT EXISTS leave_used_adjustments (
    user_id       UUID NOT NULL REFERENCES users (id)       ON DELETE CASCADE,
    leave_type_id UUID NOT NULL REFERENCES leave_types (id) ON DELETE CASCADE,
    year          INTEGER NOT NULL,
    days          DOUBLE PRECISION NOT NULL CHECK (days > 0),
    set_by        UUID REFERENCES users (id) ON DELETE SET NULL,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, leave_type_id, year)
);

-- Every change HR makes, append-only, so "who recorded those days, and what was it before?" always has an
-- answer (the adjustment row above is overwritten, and deleted when it goes back to 0; audit_logs has no
-- detail column). One row per save that changed the total.
CREATE TABLE IF NOT EXISTS leave_used_history (
    id            UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id       UUID NOT NULL REFERENCES users (id)       ON DELETE CASCADE,
    leave_type_id UUID NOT NULL REFERENCES leave_types (id) ON DELETE CASCADE,
    year          INTEGER NOT NULL,
    approved_days DOUBLE PRECISION NOT NULL,   -- what approved requests added up to at the time
    old_used      DOUBLE PRECISION NOT NULL,   -- the total before
    new_used      DOUBLE PRECISION NOT NULL,   -- the total HR set
    set_by        UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_leave_used_history_user ON leave_used_history (user_id, year);
