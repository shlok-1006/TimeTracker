-- 0059_interval_received_at.sql — when the server received each interval.
--
-- The desktop records time locally and syncs it later (Rule 1). For someone on
-- a VPN that blocks the app, "later" can be the next day or the next week, after
-- the nightly attendance rollup and AI review have already run for that day.
-- `received_at` lets us see how late a user's data arrives (start_utc vs
-- received_at) and find days that were judged without it.
--
-- Nullable with no backfill: rows from before this migration have no honest
-- receive time, and stamping them all with the migration time would make every
-- old interval look late.

ALTER TABLE intervals ADD COLUMN IF NOT EXISTS received_at TIMESTAMPTZ;
ALTER TABLE intervals ALTER COLUMN received_at SET DEFAULT now();
