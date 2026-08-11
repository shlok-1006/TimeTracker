-- 0047_deactivate_not_delete.sql — removing someone should not destroy what they did.
--
-- `DELETE /admin/users/:id` really deleted: the row went, and every FK cascaded
-- with it — intervals, screenshots, attendance days, analysis reports, tasks,
-- leave, the lot. `alumni` (0025) softened that by COPYING name/email/role/team
-- into a separate table with no foreign key, precisely because the user it
-- described no longer existed.
--
-- So the record of who worked here survived, and the record of their work did
-- not. A month-end report covering a period someone left in silently lost their
-- hours; an audit of last quarter could not explain its own numbers.
--
-- Deactivation instead. The row stays, everything joined to it stays, and the
-- person simply stops being current: they cannot sign in, they leave the
-- rosters, and they stop accruing 'absent' days. Reactivation is then just
-- clearing a timestamp, which is what makes "they're back" a one-click fact
-- rather than a re-creation that starts their history over.
--
-- The `alumni` table is NOT dropped: it holds the people already deleted under
-- the old behaviour, whose rows are genuinely gone and cannot be recovered. It
-- stays as the historical record for exactly those, and new departures land
-- here instead.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS deactivated_at TIMESTAMPTZ,
    -- Who did it. Removing someone's access is the kind of act that needs a name
    -- against it, and the audit log alone is easy to lose track of at a glance.
    ADD COLUMN IF NOT EXISTS deactivated_by UUID REFERENCES users (id) ON DELETE SET NULL;

-- Every roster read is "the active ones", so index that rather than the column:
-- a partial index over the live set stays small as leavers accumulate.
CREATE INDEX IF NOT EXISTS idx_users_active
    ON users (id)
    WHERE deactivated_at IS NULL;

-- And the alumni view is the complement, ordered by when they left.
CREATE INDEX IF NOT EXISTS idx_users_deactivated_at
    ON users (deactivated_at DESC)
    WHERE deactivated_at IS NOT NULL;
