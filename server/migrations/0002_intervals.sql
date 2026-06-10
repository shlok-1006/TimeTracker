-- 0002_intervals.sql — synced time intervals (Rule 2: immutable intervals,
-- Rule 3: UTC). The desktop app is the source of truth; rows arrive via the
-- sync worker (Rule 4) and are never mutated server-side.

CREATE TABLE IF NOT EXISTS intervals (
    -- Client-generated UUID (the desktop owns the id) so syncing is idempotent.
    id         UUID PRIMARY KEY,
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    start_utc  TIMESTAMPTZ NOT NULL,
    end_utc    TIMESTAMPTZ NOT NULL,
    idle       BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT intervals_ordered CHECK (end_utc >= start_utc)
);

CREATE INDEX IF NOT EXISTS idx_intervals_user       ON intervals (user_id);
CREATE INDEX IF NOT EXISTS idx_intervals_user_start ON intervals (user_id, start_utc);

-- Enforce immutability: intervals may be inserted (and deleted by retention),
-- but never updated.
CREATE OR REPLACE FUNCTION reject_interval_update() RETURNS trigger AS $$
BEGIN
    RAISE EXCEPTION 'intervals are immutable';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS intervals_immutable ON intervals;
CREATE TRIGGER intervals_immutable
    BEFORE UPDATE ON intervals
    FOR EACH ROW EXECUTE FUNCTION reject_interval_update();
