-- 0024_ticket_requests_invalidate_legacy_tokens.sql — SEC-28 follow-up (RA-02).
--
-- Decision tokens are now stored SHA-256-hashed (64 lowercase hex chars). Any
-- row created before that change still holds a raw *plaintext* token, which is
-- (a) no longer decidable via the new hash lookup and (b) a usable secret at
-- rest. Neutralize both: reject legacy pending requests, then overwrite every
-- non-hash token with a per-row marker (the UNIQUE constraint forbids a shared
-- constant, so key it on the row id).

UPDATE ticket_requests
SET status = 'rejected', decided_at = now()
WHERE status = 'pending'
  AND decision_token !~ '^[0-9a-f]{64}$';

UPDATE ticket_requests
SET decision_token = 'legacy-invalidated-' || id
WHERE decision_token !~ '^[0-9a-f]{64}$';
