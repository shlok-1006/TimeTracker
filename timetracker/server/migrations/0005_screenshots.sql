-- 0005_screenshots.sql — screenshot METADATA only (Rule 5).
-- Bytes live in MinIO/R2; the server stores keys + timestamps, never images.

CREATE TABLE IF NOT EXISTS screenshots (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    storage_key TEXT        NOT NULL UNIQUE,
    taken_at    TIMESTAMPTZ NOT NULL,
    -- Interval the shot belongs to. Not a FK: the interval may not have synced
    -- to the server yet (Rule 4).
    interval_id UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_screenshots_user  ON screenshots (user_id);
CREATE INDEX IF NOT EXISTS idx_screenshots_taken ON screenshots (user_id, taken_at);
