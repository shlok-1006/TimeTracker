-- Tag local intervals with an activity kind (active | idle | meeting | break)
-- so the recorder can capture status-segmented time. Existing rows are active.
ALTER TABLE intervals ADD COLUMN kind TEXT NOT NULL DEFAULT 'active';
