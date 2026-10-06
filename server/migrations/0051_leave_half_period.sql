-- 0051 · Which half of the day a half-day leave covers.
--
-- Half-day leave already exists as `days = 0.5` (migration 0013 stores days as DOUBLE PRECISION
-- precisely so a half is representable). What it never recorded is WHICH half — so an approver
-- reading "0.5 days on 12 Oct" cannot tell a morning absence from an afternoon one, and neither
-- can anyone planning cover.
--
--   half_period   NULL   = a full day, or a legacy half-day booked before this column existed
--                 'first'  = first half of the day (morning)
--                 'second' = second half (afternoon)
--
-- Deliberately NOT a per-day table: a request carries at most ONE half period, which is why the
-- constraint below pins it to a single-day, half-day request. A range like "Mon full + Tue
-- first-half" is not expressible and is not meant to be — that is two requests today.
--
-- The column is nullable with no default, so every existing row stays valid and any client that
-- has not been updated (the desktop app, employee-web) keeps booking half days exactly as before.
-- Additive and idempotent.

ALTER TABLE leave_requests
    ADD COLUMN IF NOT EXISTS half_period TEXT
        CHECK (half_period IN ('first', 'second'));

-- A half period is only meaningful on a one-day, half-day request. Float equality is safe here:
-- 0.5 is exact in binary floating point, and leave_service rounds to exactly 0.5 before insert.
--
-- ADD CONSTRAINT has no IF NOT EXISTS, so guard on the catalogue to keep the file re-runnable.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'leave_requests'::regclass
          AND conname  = 'leave_requests_half_period_scope'
    ) THEN
        ALTER TABLE leave_requests
            ADD CONSTRAINT leave_requests_half_period_scope
            CHECK (
                half_period IS NULL
                OR (days = 0.5 AND start_date = end_date)
            );
    END IF;
END
$$;
