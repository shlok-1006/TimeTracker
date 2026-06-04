-- 0003_presence.sql — live presence / status (STEP 3).
--
-- One row per user, upserted by the heartbeat (POST /presence every ~45s).
-- The stored status is what the client reports (working | idle | break);
-- `not_logged_in` is DERIVED at read time when the heartbeat goes stale.

DO $$
BEGIN
    CREATE TYPE presence_status AS ENUM ('working', 'idle', 'break', 'not_logged_in');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END
$$;

CREATE TABLE IF NOT EXISTS presence (
    user_id             UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    status              presence_status NOT NULL,
    last_seen_at        TIMESTAMPTZ     NOT NULL DEFAULT now(),
    -- The interval currently being recorded, if any. Not a FK: the interval may
    -- not have synced to the server yet (Rule 4), so we don't constrain it.
    current_interval_id UUID
);

CREATE INDEX IF NOT EXISTS idx_presence_last_seen ON presence (last_seen_at);
