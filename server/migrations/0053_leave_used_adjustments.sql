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
-- instead, so every recorded leave stays explainable. A row whose difference becomes 0 is deleted.

CREATE TABLE IF NOT EXISTS leave_used_adjustments (
    user_id       UUID NOT NULL REFERENCES users (id)       ON DELETE CASCADE,
    leave_type_id UUID NOT NULL REFERENCES leave_types (id) ON DELETE CASCADE,
    year          INTEGER NOT NULL,
    days          DOUBLE PRECISION NOT NULL CHECK (days > 0),
    set_by        UUID REFERENCES users (id) ON DELETE SET NULL,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, leave_type_id, year)
);
