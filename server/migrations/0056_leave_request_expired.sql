-- 0056_leave_request_expired.sql — a pending leave request nobody decided in time EXPIRES.
--
-- A request still `pending` some days (LEAVE_PENDING_EXPIRY_DAYS, default 3 — OKF LV-13) after the later of its
-- leave's end date and the day it was submitted (both as IST dates) is marked `expired` by the server's leave-expiry job (src/leave_expiry.rs): it leaves the approvals queue
-- and shows as Expired in the employee's history. It is not deleted — the record stays — and nothing about
-- the balance changes (only approved requests count as used). `decided_at` records when it expired;
-- `approver_id` stays NULL (no person decided it).
--
-- The grace days exist because leave is often applied for AFTER the fact (sick leave): such a request must
-- still reach HR before it expires — hence they also count from the submit day, not only from the end date.

ALTER TABLE leave_requests DROP CONSTRAINT IF EXISTS leave_requests_status_check;
ALTER TABLE leave_requests ADD CONSTRAINT leave_requests_status_check
    CHECK (status IN ('pending', 'approved', 'rejected', 'cancelled', 'expired'));

-- The job's lookup: pending requests by end date (the job also tests `end_date < cutoff`, which the
-- submit-day rule implies, so this index still narrows the scan).
CREATE INDEX IF NOT EXISTS idx_leave_requests_pending_end ON leave_requests (end_date) WHERE status = 'pending';
