-- 0004_activity.sql — local activity aggregates (Activity feature).
--
-- The sampler accumulates two aggregates while the user is Working. Privacy:
-- app NAMES only — no window titles, and never any keystroke content; input
-- is observed only as "was there any input in this sample?".
--
-- Rows are mutable counters (unlike immutable intervals), so sync uses a
-- dirty flag instead of a synced-once queue: every accumulation sets
-- dirty = 1; the sync worker posts absolute values and clears dirty only if
-- the row wasn't touched meanwhile (guarded by updated_at).

CREATE TABLE IF NOT EXISTS app_usage (
    day        TEXT NOT NULL,             -- YYYY-MM-DD (UTC)
    app_name   TEXT NOT NULL,
    seconds    INTEGER NOT NULL DEFAULT 0,
    dirty      INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL,             -- RFC3339
    PRIMARY KEY (day, app_name)
);

CREATE TABLE IF NOT EXISTS activity_blocks (
    block_start    TEXT PRIMARY KEY,      -- RFC3339, 10-minute UTC boundary
    active_seconds INTEGER NOT NULL DEFAULT 0,
    total_seconds  INTEGER NOT NULL DEFAULT 0,
    dirty          INTEGER NOT NULL DEFAULT 1,
    updated_at     TEXT NOT NULL
);
