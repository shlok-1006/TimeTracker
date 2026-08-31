-- 0048_task_pr_links.sql — link GitHub PRs to a manual task.
--
-- The HRMS performance engine scores a person by reviewing the PRs attached to their tasks. A task
-- can carry several PRs (one feature, many PRs), so this is an array of full PR URLs — the URL
-- carries the repo, so nothing needs a naming convention. Title/description are untouched (the
-- screenshot analyzer still reads them). Empty array = no PRs = not scored by the engine.

ALTER TABLE manual_tasks
    ADD COLUMN IF NOT EXISTS pr_links TEXT[] NOT NULL DEFAULT '{}';

-- The engine pulls "PR-bearing tasks for a team", so index the has-a-PR predicate.
CREATE INDEX IF NOT EXISTS idx_manual_tasks_has_pr
    ON manual_tasks (user_id)
    WHERE cardinality(pr_links) > 0;
