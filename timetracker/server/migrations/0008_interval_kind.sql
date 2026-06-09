-- 0008_interval_kind.sql — tag intervals with an activity kind so the admin
-- timeline can show active / idle / meeting / break segments.
-- Existing rows are active work (default). `idle` column is retained but
-- superseded by `kind` (idle == kind 'idle').
ALTER TABLE intervals ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'active';

DO $$
BEGIN
    ALTER TABLE intervals
        ADD CONSTRAINT intervals_kind_valid
        CHECK (kind IN ('active', 'idle', 'meeting', 'break'));
EXCEPTION
    WHEN duplicate_object THEN NULL;
END
$$;
