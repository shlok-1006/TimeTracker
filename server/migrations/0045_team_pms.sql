-- 0045_team_pms.sql — who runs a team, as a fact rather than an inference.
--
-- TimeTracker has only ever known management as a person-to-person relation
-- (`user_managers`), so "this team's PMs" has been DERIVED: the managers of the
-- team's members. That reading is honest about the data we hold and wrong about
-- the org. It cannot express a PM who manages nobody on the team, it makes a
-- member with two managers contribute both to a PM set nobody chose, and it has
-- no way to say "these two run this team" at all.
--
-- The HRMS needs it as a fact (their 11 Aug note, §2): a PM can run two teams,
-- a team can have several PMs, and the same two people can sit on two teams
-- whose work scores against different repos. None of that is a reporting line.
--
-- Deliberately NOT a role column on `user_teams`. That table is read in 15 places
-- across 4 files, and adding a role would silently change what every one of them
-- means — quietly counting PMs as staff in headcounts, rosters and, worst,
-- team attendance. It would also force "PM of X ⇒ member of X", when the very
-- case being modelled is 2 PMs over 5 members: two different sets.
--
-- Same name and columns as the HRMS's interim `sql/292_team_pms.sql`, whose own
-- comment calls it "interim until Shlok exposes a first-class team_pms" — theirs
-- becomes a cache of this rather than a second opinion.

CREATE TABLE IF NOT EXISTS team_pms (
    team_id    UUID NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    pm_user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Who assigned them. Scope is a permission, and a permission with no author
    -- is the kind of thing nobody can explain six months later.
    added_by   UUID REFERENCES users (id) ON DELETE SET NULL,
    added_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (team_id, pm_user_id)
);

-- The lookup on every scoped team read: "which teams may this PM see?"
CREATE INDEX IF NOT EXISTS idx_team_pms_pm ON team_pms (pm_user_id);

-- ── Seed: reproduce today's derived answer as real rows ─────────────────────
--
-- Without this, the moment `pms[]` starts reading this table every team reports
-- NO PMs until someone assigns them — the HRMS renders that field already, so
-- their UI would empty out with no error at all. Seeding means the field looks
-- unchanged on day one and starts being *correct* the first time HR edits it.
--
-- `added_by` is NULL on purpose: nobody decided these, they were inferred. That
-- distinction is worth keeping — a NULL author says "this came from the old
-- derivation", which is exactly the row you want to find later.
INSERT INTO team_pms (team_id, pm_user_id, added_by)
SELECT DISTINCT ut.team_id, um.manager_id, NULL::uuid
FROM user_teams ut
JOIN user_managers um ON um.user_id = ut.user_id
ON CONFLICT DO NOTHING;
