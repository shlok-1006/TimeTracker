-- Task weight (importance, 1–10) and an optional expected due date, both set by
-- the HR / project manager who assigns the task. Existing rows default to a
-- neutral weight of 5 and no due date.
ALTER TABLE manual_tasks
    ADD COLUMN weight   INTEGER NOT NULL DEFAULT 5 CHECK (weight BETWEEN 1 AND 10),
    ADD COLUMN due_date DATE;
