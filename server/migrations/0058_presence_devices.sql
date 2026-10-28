-- 0058_presence_devices.sql — one live status per device, one badge per person.
--
-- `presence` keeps a single row per person and every heartbeat overwrites it, so someone
-- signed in on two machines — the laptop on a break, the desktop in active use — had their
-- badge flip between "On break" and "Working" every ~45 seconds, whichever machine beat last.
-- The hours were already right (migration 0044 unions the devices); only the badge wasn't.
--
-- Working on two devices at once is allowed, so the badge describes the PERSON: if any of
-- their devices is working, they are working. Each device now keeps its own row here and the
-- badge is the best status among the fresh ones, in the same order the hours use:
--
--     meeting  >  working  >  idle  >  break  >  not_working
--
-- DEVICE KEY. Desktop builds that send a `device_id` (a random id minted once per install)
-- are keyed `device:<id>`, so a switch on that machine shows at once. Older builds send no
-- id, and two of them cannot be told apart; their heartbeats are keyed by the STATUS they
-- report instead (`legacy:working`, `legacy:break`, …). Taking the best fresh one still
-- stops the flicker. The cost is that a single old-build device switching from working
-- to break keeps showing "Working" until that row goes stale (the 90-second grace). That
-- ends as each machine updates.
--
-- `presence` itself is left as it was: it still records the last heartbeat (last_seen_at,
-- current_interval_id) for whoever reads it.

CREATE TABLE IF NOT EXISTS presence_devices (
    user_id      UUID            NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    device_key   TEXT            NOT NULL,
    status       presence_status NOT NULL,
    last_seen_at TIMESTAMPTZ     NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, device_key),
    CONSTRAINT presence_devices_key_shape CHECK (device_key ~ '^(device|legacy):[a-z0-9_-]{1,64}$'),
    -- Server-derived only, exactly as on `presence`.
    CONSTRAINT presence_devices_reportable CHECK (status <> 'not_logged_in')
);

-- Higher wins when one person's devices disagree. `not_logged_in` is never stored here.
CREATE OR REPLACE FUNCTION presence_rank(s presence_status)
RETURNS INTEGER
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT CASE s
        WHEN 'meeting'     THEN 5
        WHEN 'working'     THEN 4
        WHEN 'idle'        THEN 3
        WHEN 'break'       THEN 2
        WHEN 'not_working' THEN 1
        ELSE 0
    END
$$;

-- The person's live status: the best status among devices heard from within the grace
-- window, or `not_logged_in` when none has been.
CREATE OR REPLACE FUNCTION presence_live_status(p_user UUID, p_grace_secs DOUBLE PRECISION)
RETURNS TEXT
LANGUAGE sql STABLE PARALLEL SAFE
AS $$
    SELECT COALESCE(
        (SELECT d.status::text
         FROM presence_devices d
         WHERE d.user_id = p_user
           AND d.last_seen_at >= now() - make_interval(secs => p_grace_secs)
         ORDER BY presence_rank(d.status) DESC
         LIMIT 1),
        'not_logged_in')
$$;

-- Carry the current heartbeats over, so nobody's badge drops to "Not logged in" for a beat
-- when this ships. A stale one just stays stale.
INSERT INTO presence_devices (user_id, device_key, status, last_seen_at)
SELECT user_id, 'legacy:' || status::text, status, last_seen_at
FROM presence
WHERE status <> 'not_logged_in'
ON CONFLICT (user_id, device_key) DO NOTHING;
