-- 0006_not_working_status.sql — "not_working" = logged in but timer stopped
-- (distinct from "idle", which is tracking with no input).
ALTER TYPE presence_status ADD VALUE IF NOT EXISTS 'not_working';
