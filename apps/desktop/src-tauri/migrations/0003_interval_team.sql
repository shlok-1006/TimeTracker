-- Tag local intervals with the team the session was logged under (Feature 4).
-- Nullable: an employee with no teams (or legacy rows) has no team_id. Synced to
-- the server's intervals.team_id via the sync queue.
ALTER TABLE intervals ADD COLUMN team_id TEXT;
