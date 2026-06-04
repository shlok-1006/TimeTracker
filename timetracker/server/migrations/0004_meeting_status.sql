-- 0004_meeting_status.sql — add "meeting" to the presence status set.
-- (PostgreSQL 12+ permits ADD VALUE inside a migration transaction.)
ALTER TYPE presence_status ADD VALUE IF NOT EXISTS 'meeting';
