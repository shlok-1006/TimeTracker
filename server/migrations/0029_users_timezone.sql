-- 0029_users_timezone.sql — per-user IANA timezone.
--
-- Used only to bucket the "today's / this week's hours" display at a 04:00
-- LOCAL business-day boundary (so late-night work counts toward the day it
-- began, and the desktop's local figure matches the admin's server figure).
-- Reported by the desktop app; NULL until then, in which case the hours window
-- falls back to UTC. Attendance and daily reports remain on UTC days.

ALTER TABLE users ADD COLUMN IF NOT EXISTS timezone TEXT;
