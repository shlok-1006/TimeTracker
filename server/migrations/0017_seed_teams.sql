-- 0017_seed_teams.sql — seed the organisation's standing teams (Feature 4).
-- Idempotent: re-running (or a name already created by HR) is a no-op.

INSERT INTO teams (name, description) VALUES
    ('Agent Builder',  ''),
    ('Customer Issues', ''),
    ('HR & Marketing',  ''),
    ('Ruh Platform',    ''),
    ('SDR',             ''),
    ('Tech Tasks',      '')
ON CONFLICT (name) DO NOTHING;
