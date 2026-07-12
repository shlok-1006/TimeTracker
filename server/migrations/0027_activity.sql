-- 0027_activity.sql — Activity feature (app usage + input-activity levels).
--
-- Two aggregates synced up from the desktop (which owns the raw sampling and
-- keeps only aggregates too — no keystrokes, no window titles, app NAMES only):
--
--   app_usage        one row per (user, UTC day, app): foreground seconds.
--   activity_blocks  one row per (user, 10-minute UTC block): seconds with
--                    real keyboard/mouse input vs seconds tracked.
--
-- Counters are MONOTONIC within their window: the client accumulates locally
-- and syncs absolute values, and the server upsert takes GREATEST(old, new),
-- so at-least-once delivery (retries, restarts) can never double-count.

CREATE TABLE IF NOT EXISTS app_usage (
    id         UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day        DATE NOT NULL,
    app_name   TEXT NOT NULL,
    seconds    INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, day, app_name)
);

CREATE INDEX IF NOT EXISTS idx_app_usage_user_day ON app_usage (user_id, day);

CREATE TABLE IF NOT EXISTS activity_blocks (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id        UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Start of the 10-minute UTC window this block covers.
    block_start    TIMESTAMPTZ NOT NULL,
    active_seconds INTEGER NOT NULL DEFAULT 0,
    total_seconds  INTEGER NOT NULL DEFAULT 0,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, block_start),
    CHECK (active_seconds <= total_seconds)
);

CREATE INDEX IF NOT EXISTS idx_activity_blocks_user ON activity_blocks (user_id, block_start);
