-- 0016_teams.sql — Teams + multi-team membership (Feature 4).
--
-- An employee may belong to MANY teams (user_teams is a join table). Tracked
-- intervals carry the team the work was logged under, so time can be attributed
-- per team. (Supersedes the single users.manager_id for team grouping; manager_id
-- is retained for the existing PM-scope/approval logic.)

CREATE TABLE IF NOT EXISTS teams (
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Many-to-many: one employee in many teams; one team has many employees.
CREATE TABLE IF NOT EXISTS user_teams (
    user_id  UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    team_id  UUID NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    added_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, team_id)
);

CREATE INDEX IF NOT EXISTS idx_user_teams_team ON user_teams (team_id);

-- Attribute each tracked interval to the team it was logged under (nullable:
-- legacy rows and untagged sessions have no team).
ALTER TABLE intervals
    ADD COLUMN IF NOT EXISTS team_id UUID REFERENCES teams (id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS idx_intervals_team ON intervals (team_id);
