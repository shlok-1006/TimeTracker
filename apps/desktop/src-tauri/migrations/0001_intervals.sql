-- Local SQLite schema (Rule 1: local-first source of truth).
--
-- `intervals` matches the STEP 2 spec exactly (id, user_id, start_utc, end_utc,
-- idle) and is immutable (Rule 2) — enforced by a trigger that blocks UPDATE.
-- Timestamps are RFC3339 UTC strings (Rule 3).
--
-- Sync bookkeeping lives in a SEPARATE table so `intervals` stays immutable and
-- append-only (Rule 4: SQLite -> sync queue -> API). An interval is "pending"
-- until a row exists for it in `interval_sync`.

CREATE TABLE IF NOT EXISTS intervals (
    id        TEXT PRIMARY KEY NOT NULL,
    user_id   TEXT NOT NULL,
    start_utc TEXT NOT NULL,
    end_utc   TEXT NOT NULL,
    idle      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_intervals_user ON intervals (user_id);

CREATE TRIGGER IF NOT EXISTS intervals_immutable
BEFORE UPDATE ON intervals
BEGIN
    SELECT RAISE(ABORT, 'intervals are immutable');
END;

CREATE TABLE IF NOT EXISTS interval_sync (
    interval_id TEXT PRIMARY KEY NOT NULL REFERENCES intervals (id),
    synced_at   TEXT NOT NULL
);
