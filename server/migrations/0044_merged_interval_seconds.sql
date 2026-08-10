-- 0044_merged_interval_seconds.sql — count each wall-clock second once.
--
-- Rule 2 says totals are derived from intervals, never kept as a counter. They are,
-- but every derivation so far has been a plain SUM(end_utc - start_utc), which is
-- only correct while a person's intervals never overlap.
--
-- They can. Interval ids are minted by the desktop app and the sync insert dedupes
-- on that id alone (`ON CONFLICT (id) DO NOTHING`), so two installs signed in as the
-- same person each write their own minute rows for the same minute. Nothing rejects
-- them, and every total downstream counts that minute twice. Observed in production
-- on 10 Aug 2026: one person's day summed to 10h36m across a 6h16m span — 1.71x —
-- which flowed into the attendance rollup, and from there into the weekly 40-hour
-- compliance check and the monthly report.
--
-- Deleting the "extra" rows is the wrong fix: both recordings genuinely happened, we
-- would be guessing which to destroy, and the next double-install would do it again.
-- The right fix is to derive honestly — union the time rather than add it up.
--
-- PRECEDENCE. Two recorders can label the same second differently, so a second is
-- attributed to exactly one kind, most-specific-and-most-positive first:
--
--     meeting  >  active  >  idle  >  break
--
-- meeting is an explicit mode, active is observed input, idle is the absence of it,
-- and break is one device being paused — a device pausing does not unmake input another
-- device saw. This only matters where two recorders disagree, which is itself the
-- anomaly; with a single recorder every kind is already disjoint and this changes
-- nothing at all.

-- Length of a multirange, in seconds. Multiranges are already normalised (overlapping
-- members are coalesced on construction), so this is a plain sum over the pieces.
CREATE OR REPLACE FUNCTION tstz_multirange_seconds(mr tstzmultirange)
RETURNS BIGINT
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT COALESCE(SUM(EXTRACT(EPOCH FROM (upper(r) - lower(r))))::bigint, 0)
    FROM unnest(mr) AS r
$$;

-- One user's activity in [p_from, p_to), overlap-free and split by kind.
--
-- Every returned second is counted once and belongs to exactly one column, so
-- `active + idle + meeting` IS the worked-plus-idle total rather than an upper bound
-- on it. Callers that want "worked" use active + meeting, as they did before.
--
-- Intervals are clipped to the window first, so a session spanning midnight
-- contributes only its part of the day — the same LEAST/GREATEST clipping the
-- attendance rollup already did.
-- `p_team` NULL means "any team, and untagged work too"; a team id restricts to work
-- logged under it. Required rather than defaulted, so there is one function and no
-- chance of an ambiguous overload as call sites are added.
CREATE OR REPLACE FUNCTION interval_seconds(
    p_user UUID,
    p_from TIMESTAMPTZ,
    p_to   TIMESTAMPTZ,
    p_team UUID
)
RETURNS TABLE (active BIGINT, idle BIGINT, meeting BIGINT, brk BIGINT)
LANGUAGE sql STABLE PARALLEL SAFE
AS $$
    WITH clipped AS (
        SELECT kind,
               tstzrange(GREATEST(start_utc, p_from), LEAST(end_utc, p_to), '[)') AS r
        FROM intervals
        WHERE user_id = p_user
          AND end_utc > p_from
          AND start_utc < p_to
          AND (p_team IS NULL OR team_id = p_team)
          -- A zero-length clip contributes nothing and would only make an empty range.
          AND GREATEST(start_utc, p_from) < LEAST(end_utc, p_to)
    ),
    -- range_agg coalesces overlaps within a kind: this is where the double count dies.
    by_kind AS (
        SELECT kind, range_agg(r) AS m FROM clipped GROUP BY kind
    ),
    pick AS (
        SELECT
            COALESCE((SELECT m FROM by_kind WHERE kind = 'meeting'), '{}'::tstzmultirange) AS mtg,
            COALESCE((SELECT m FROM by_kind WHERE kind = 'active'),  '{}'::tstzmultirange) AS act,
            COALESCE((SELECT m FROM by_kind WHERE kind = 'idle'),    '{}'::tstzmultirange) AS idl,
            COALESCE((SELECT m FROM by_kind WHERE kind = 'break'),   '{}'::tstzmultirange) AS brk
    ),
    -- Subtract what a higher-precedence kind already claimed, so the four are disjoint.
    layered AS (
        SELECT mtg,
               act - mtg                   AS act,
               idl - mtg - act             AS idl,
               brk - mtg - act - idl       AS brk
        FROM pick
    )
    SELECT tstz_multirange_seconds(act),
           tstz_multirange_seconds(idl),
           tstz_multirange_seconds(mtg),
           tstz_multirange_seconds(brk)
    FROM layered
$$;

-- The window predicate above is the hot path for every hours/attendance read.
CREATE INDEX IF NOT EXISTS idx_intervals_user_window
    ON intervals (user_id, start_utc, end_utc);
