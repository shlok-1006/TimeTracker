-- 0009_linear_links.sql — map internal users to their Linear account.
-- The Linear API token is NOT stored here (it lives in server config / env);
-- this only records which Linear user an employee corresponds to.

CREATE TABLE IF NOT EXISTS linear_links (
    user_id        UUID PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    linear_user_id TEXT        NOT NULL,
    linked_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
