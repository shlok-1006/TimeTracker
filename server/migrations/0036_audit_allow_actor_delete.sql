-- Let a user be deleted even when they have audit history.
--
-- `audit_logs.actor_id` is `ON DELETE SET NULL`, but the immutability trigger
-- (`reject_audit_mutation`, migration 0001) rejected UPDATEs too — so deleting
-- ANY user who had ever performed an audited action (login, task, etc.) failed
-- with "audit_logs are immutable", and the whole cascade aborted. That's why HR
-- couldn't delete users from the dashboard.
--
-- Keep the log's CONTENT immutable, but permit exactly the FK-driven case:
-- setting `actor_id` from non-null to NULL with every other column unchanged.
-- All other updates and all deletes stay blocked.
CREATE OR REPLACE FUNCTION reject_audit_mutation() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        RAISE EXCEPTION 'audit_logs are immutable';
    END IF;
    -- Allow ON DELETE SET NULL to null the actor reference (user deletion).
    IF OLD.actor_id IS NOT NULL
       AND NEW.actor_id IS NULL
       AND NEW.id          = OLD.id
       AND NEW.action      = OLD.action
       AND NEW.entity_type = OLD.entity_type
       AND NEW.entity_id   IS NOT DISTINCT FROM OLD.entity_id
       AND NEW.created_at  = OLD.created_at THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'audit_logs are immutable';
END;
$$ LANGUAGE plpgsql;
