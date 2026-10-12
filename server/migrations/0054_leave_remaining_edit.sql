-- 0054_leave_remaining_edit.sql — HR edits REMAINING leave; every change is kept.
--
-- 0053 let HR type how many days someone had USED. The People → Leaves screen now asks for what HR actually
-- knows from the old records: how many days are LEFT. Nothing about the storage changes — the server turns
-- "remaining" into used = allotted − remaining under the leave-approval lock and stores the days used outside
-- TimeTracker in leave_used_adjustments exactly as before (POST /admin/leave/used, `remaining` or `used`).
--
-- Storing USED rather than a pinned remaining figure is the point: a leave approved in TimeTracker afterwards
-- is one more approved request, so remaining keeps going down by the days approved, with no second write.
--
-- What this migration adds is the history. leave_used_adjustments is overwritten (and deleted when it goes
-- back to 0), and audit_logs has no detail column, so "who changed this person's balance, and from what?"
-- had no answer. One append-only row per save that changed the balance.

CREATE TABLE IF NOT EXISTS leave_used_history (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id        UUID NOT NULL REFERENCES users (id)       ON DELETE CASCADE,
    leave_type_id  UUID NOT NULL REFERENCES leave_types (id) ON DELETE CASCADE,
    year           INTEGER NOT NULL,
    allotted_days  DOUBLE PRECISION NOT NULL,   -- the allotment at the time
    approved_days  DOUBLE PRECISION NOT NULL,   -- what approved requests added up to at the time
    old_used       DOUBLE PRECISION NOT NULL,   -- used before (approved + recorded)
    new_used       DOUBLE PRECISION NOT NULL,   -- used after
    set_by         UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_leave_used_history_user ON leave_used_history (user_id, year);
