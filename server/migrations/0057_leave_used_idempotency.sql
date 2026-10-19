-- 0057_leave_used_idempotency.sql — POST /admin/leave/used can ADD days, and a caller can make a call safe to retry.
--
-- HRMS deducts half days of leave for short working days. With only absolute targets (used_days /
-- remaining_days) it had to read "used", add 0.5 and write the sum back: two deductions at once lost one, a leave
-- approval landing between the read and the write cancelled the deduction, and a retry after a lost response
-- deducted twice. The endpoint now takes `add_used_days` (applied under the leave-approval lock, on top of the
-- days already recorded) and an optional `idempotency_key`.
--
-- The key lives on leave_used_history rather than in a table of its own: the history row is already written in
-- the same transaction, under the same per-person lock, as the balance change it records, so "the change was
-- applied" and "the key was used" are one row and can't disagree. A keyed save that changes nothing (an absolute
-- target equal to the current balance) still writes its history row (old_used = new_used) so the key is
-- remembered and a retry replays instead of re-applying.
--
-- The index is unique across ALL people, types and years: a key reused for a different person, type, year or
-- request is refused (409), and two concurrent calls with the same key can't both commit — the second waits on the
-- index for the first and then sees its row.
--
-- idempotency_request records what the call asked for ("add_used_days=0.5", "used_days=3", …), so a reused key
-- with a different amount is refused too rather than silently replayed.

ALTER TABLE leave_used_history ADD COLUMN IF NOT EXISTS idempotency_key     TEXT;
ALTER TABLE leave_used_history ADD COLUMN IF NOT EXISTS idempotency_request TEXT;

ALTER TABLE leave_used_history DROP CONSTRAINT IF EXISTS leave_used_history_idempotency_key_format;
ALTER TABLE leave_used_history ADD CONSTRAINT leave_used_history_idempotency_key_format
    CHECK (idempotency_key IS NULL OR idempotency_key ~ '^[A-Za-z0-9:_-]{1,200}$');

ALTER TABLE leave_used_history DROP CONSTRAINT IF EXISTS leave_used_history_idempotency_pair;
ALTER TABLE leave_used_history ADD CONSTRAINT leave_used_history_idempotency_pair
    CHECK ((idempotency_key IS NULL) = (idempotency_request IS NULL));

CREATE UNIQUE INDEX IF NOT EXISTS uq_leave_used_history_idempotency_key
    ON leave_used_history (idempotency_key)
    WHERE idempotency_key IS NOT NULL;
