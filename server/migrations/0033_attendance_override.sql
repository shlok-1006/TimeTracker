-- Manual HR override of a derived attendance status. `is_override` protects the
-- row from the nightly rollup and the "always recompute today" path — the
-- derived upsert skips rows where it is true — so an HR edit survives. Clearing
-- the override lets derivation take over again. `overridden_by` records the HR
-- user who last edited the row (audit-friendly; nulled if that account is gone).
ALTER TABLE attendance_days
    ADD COLUMN is_override   BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN overridden_by UUID REFERENCES users (id) ON DELETE SET NULL;
