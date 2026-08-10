-- 0043_profile_form_source.sql — which form response produced a profile.
--
-- The onboarding sync (RUH-HRMS: app/api/directory/sync-form) is a PULL: it reads
-- every response and writes each person's record. That is safe as a deliberate
-- act, but it is about to run on a schedule, and `upsert_profile` replaces every
-- column while `replace_education` deletes and re-inserts. On a timer that turns
-- any HR correction — a fixed typo, a corrected joining date — into something the
-- next tick silently reverts to whatever the form said.
--
-- The fix is a watermark rather than a rule about who may edit what: record WHICH
-- response produced the row, and let the sync skip anyone whose latest response is
-- one it has already applied. A genuine re-submission still lands (new id, newer
-- timestamp); a steady-state run writes nothing at all.
--
-- Nullable on purpose: the seven profiles imported by hand before this existed are
-- real records, not broken ones. A NULL here means "we don't know which response
-- this came from", and the sync treats that as stale — so the first scheduled run
-- refreshes them once and pins them from then on.

ALTER TABLE employee_profiles
    -- Google's opaque response id. Text, not UUID: it is Google's identifier and
    -- its shape is theirs to change.
    ADD COLUMN IF NOT EXISTS form_response_id  TEXT,
    -- The submission time as Google reported it. Kept alongside the id so a
    -- re-submission is orderable, not merely different.
    ADD COLUMN IF NOT EXISTS form_submitted_at TIMESTAMPTZ;
