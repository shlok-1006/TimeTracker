# TimeTracker — Database Schema & Data Flow

PostgreSQL (Supabase in production). Every table below lives in the single
`public` schema and is created by the migrations in `server/migrations/`
(applied automatically at server startup). The desktop app additionally keeps
its own **local SQLite** database — the server never sees it directly; data
arrives only through the sync API.

## Conventions (enforced across the schema)

- **UTC everywhere** — every timestamp is `TIMESTAMPTZ` stored in UTC; local
  time is a UI concern only.
- **UUID primary keys** (`uuid_generate_v4()`), except where the client owns
  the id (intervals — see below).
- **Derived, never mutated** — totals (hours, attendance, scores) are always
  computed from immutable base records; there is no "hours counter" anywhere.
- **Immutability triggers** — `audit_logs` rejects UPDATE/DELETE; `intervals`
  rejects UPDATE.
- **No file bytes in the DB** — screenshots and documents store a
  `storage_key` pointing at the GCS bucket; bytes never touch Postgres.
- **Roles are a Postgres enum** (`user_role`: `employee`,
  `project_manager`, `hr`) — never magic strings.

## The write path (how data gets in)

```
Desktop app ──► local SQLite (always first — Rule 1)
     │              │
     │              └── sync worker (every ~15s, at-least-once, idempotent)
     │                        │
     ▼                        ▼
 screenshot bytes      POST /api/intervals ─────────► intervals
 PUT direct to GCS     POST /api/presence  ─────────► presence (upsert)
 (presigned URL)       POST /api/screenshots ───────► screenshots (metadata)
                       POST /api/attendance… etc.

Admin dashboard ──► POST /admin/… ──► users, teams, leave, manual_tasks, …
Nightly jobs    ──► attendance_days, analysis_*, weekly_hours_reports
```

## The read path (how data gets out)

Every read is scoped by role at the API layer: employees read only their own
rows (`/me/*`), project managers read only users whose `manager_id` is them,
HR reads everything. The dashboard never queries Postgres directly — all
access goes through the Axum API with JWT auth.

---

## 1. Identity, auth & audit

### `users`
The root table — every other table hangs off `users.id`.

| Column | Type | Meaning |
|---|---|---|
| `id` | UUID PK | |
| `name`, `email` (UNIQUE) | TEXT | identity; email is the login |
| `password_hash` | TEXT | Argon2id hash — plaintext is never stored |
| `role` | `user_role` enum | `employee` / `project_manager` / `hr` |
| `manager_id` | UUID → users | which PM "owns" this user (PM scope checks) |
| `team_id` | UUID | legacy single-team pointer (superseded by `user_teams`) |
| `created_at` | TIMESTAMPTZ | account start — **attendance never predates this** |

**Fetched by:** login (`email` lookup), every auth middleware call (id from the
JWT `sub`), team scoping (`manager_id`), and the attendance clamp
(`created_at`).

### `refresh_tokens`
Rotating refresh tokens (Rule 6). Only the **SHA-256 hash** of the token is
stored — a DB leak exposes no usable secrets.

| Column | Notes |
|---|---|
| `token_hash` UNIQUE | SHA-256 of the opaque token the client holds |
| `expires_at`, `revoked_at` | rotation = revoke-on-use + issue new; reuse of a revoked token revokes **all** the user's sessions (stolen-token defense) |

**Fetched by:** `POST /auth/refresh` (hash lookup), `POST /auth/logout`
(revoke), password change/reset (revoke all).

### `audit_logs` *(immutable — trigger blocks UPDATE/DELETE)*
`actor_id, action, entity_type, entity_id, created_at`. Appended by every
sensitive action (login, user create/delete, screenshot access, analysis runs,
exports). **Fetched by:** HR-only audit views.

### `alumni`
Snapshot of a user's identity taken just before HR deletes them (the delete
cascades all their data). No FK on `user_id` — the user row is gone.
**Fetched by:** the Alumni page (HR only), newest `removed_at` first.

---

## 2. Time tracking

### `intervals` *(immutable — trigger blocks UPDATE)*
The heart of the system: one row per tracked time segment.

| Column | Notes |
|---|---|
| `id` UUID PK | **client-generated** by the desktop → sync is idempotent (`ON CONFLICT DO NOTHING`) |
| `user_id` | always taken from the JWT, never from the request body |
| `start_utc`, `end_utc` | `CHECK (end_utc >= start_utc)` |
| `kind` | `active` / `idle` / `meeting` / `break` |
| `team_id` | → teams; which team the work was logged under (nullable) |

**Written by:** the desktop sync worker (`POST /intervals`, batched).
**Fetched by:** everything that shows time — `/me/hours` and the admin
drill-down (`SUM(end_utc - start_utc)` grouped as today/week/total), the
timeline bar (`/admin/users/:id/timeline` — ordered segments in a window),
today's roster hours, attendance rollups, weekly-hours compliance, and team
attribution. Hours are **always computed in SQL from intervals at read time**.

### `presence`
One row per user (PK `user_id`), upserted by the desktop heartbeat every ~45s:
`status` (`working`/`idle`/`break`/`meeting`/`not_working`), `last_seen_at`,
`current_interval_id`.

**Fetched by:** the live roster (`GET /admin/team`). `not_logged_in` is
**derived at read time**: no row, or `last_seen_at` older than the 90s grace
period. Employees always appear in the roster; HR/PM accounts appear once they
have a presence row (i.e. actually used the tracker).

---

## 3. Screenshots

### `screenshots` *(metadata only — bytes live in GCS)*

| Column | Notes |
|---|---|
| `storage_key` UNIQUE | object path in the GCS bucket (`<user_id>/<yyyymmdd>/<uuid>.jpg`) |
| `taken_at` | capture time (UTC); indexed `(user_id, taken_at)` |
| `captured_status` | presence at capture: only `working` shots are analyzed |
| `interval_id` | soft link to the interval (no FK — it may not have synced yet) |

**Write flow (Rule 5):** desktop asks `POST /uploads/presign` → server mints a
short-lived V4-signed PUT URL (user-namespaced key) → desktop uploads bytes
**directly to GCS** → desktop posts the metadata to `POST /screenshots`.

**Read flow:** viewers get **presigned GET URLs valid for ~2 minutes** —
storage keys are never exposed. Fetched by the per-day gallery
(`/admin/users/:id/screenshots?day=` — a `[day 00:00, next-day)` UTC window
LEFT JOINed to `analysis_results` for verdict badges), `/me/screenshots`, the
sampler, and the range analyzer (`count/list _in_range` over `[from,to)`).

---

## 4. AI analysis

### `analysis_jobs` + `analysis_job_samples`
One job per `(user, day)` (UNIQUE). The sampler splits the day into 5 UTC
buckets and picks one random *working* screenshot per bucket (4–5 total),
recording them in `analysis_job_samples (job_id, screenshot_id, bucket)`.
Sampling is idempotent — a day is never re-sampled.

### `analysis_results`
One AI verdict per screenshot per job (UNIQUE `(job_id, screenshot_id)` —
re-analysis upserts): `verdict` (`aligned`/`partially_aligned`/`not_aligned`/
`inconclusive`), `matched_ticket`, `confidence`, `observed`, `rationale`,
`model`. Written by the vision analyzer (Claude) after comparing the image to
the employee's Linear tickets + manual tasks. **Fetched by:** the day gallery
(verdict badges) and report building.

### `analysis_reports`
One aggregate per `(user, day)` (UNIQUE): verdict counts, `alignment_score`
(0–100 weighted: aligned=1, partial=0.5, inconclusive excluded),
`summary_text` (AI-written), `low_score_notified_at` (stamps the one-time HR
alert so restarts never re-email). **Written by:** the nightly job (2:00 UTC)
and on-demand analysis. **Fetched by:** the per-day report card and HR/PM
roster report views.

### `analysis_range_runs`
Progress tracking for the admin "analyze every screenshot in a range" feature:
`from_utc`/`to_utc`, `status` (`running`/`completed`/`failed`), counters
(`total`/`analyzed`/`skipped`/`failed`), `requested_by`. The background task
bumps counters after each screenshot; the UI polls `GET /admin/analysis-runs/:id`
for a live progress bar. Verdicts land in the normal `analysis_results` under
each covered day's job.

---

## 5. Work context (what employees are supposed to be doing)

### `linear_links`
`user_id (PK) → linear_user_id`. Maps an employee to their Linear account
(auto-linked by matching email). The Linear API token is **not** stored — it
lives in server env. Tickets themselves are **not persisted**: they're fetched
live from Linear's GraphQL API with an in-memory 1-hour cache (stale-served on
rate limit). **Fetched by:** `/me/tickets` and the analysis context builder.

### `manual_tasks`
HR/PM-assigned internal work items (`title`, `description`, `status`
`open`/`done`). Open tasks join Linear tickets in the AI analysis context,
with ids prefixed `task:` to distinguish them.

### `ticket_requests`
Employee requests for access to a Linear ticket, decided by the ticket owner
via an emailed one-time link. `decision_token` is stored SHA-256-hashed.

### `teams` + `user_teams`
Many-to-many team membership (an employee can be in several teams).
`intervals.team_id` attributes each tracked segment to the team the work was
logged under, so hours can be reported per team. Six standing teams are
seeded. **Fetched by:** team pickers, team summaries, per-team hours.

---

## 6. Attendance, leave & compliance

### `attendance_days`
One **derived** row per user per UTC day (UNIQUE `(user_id, day)`): `status`
(`present`/`absent`/`leave`/`holiday`/`weekend` — a 2-minute tracked day is
`present`), `worked_seconds`, `idle_seconds`, `first_in_utc`, `last_out_utc`,
`note` (leave type / holiday name).

**Written by:** the nightly rollup (every employee + any HR/PM who tracked
that day) and on-demand `ensure_range` when a calendar is viewed (fills
missing past days, always refreshes today). Derivation precedence: worked time
≥ threshold → `present`; else approved leave → holiday → weekend → `absent`.
**Never derived before `users.created_at`** — that's what makes an admin
"fresh start" (bump `created_at`, delete old rows) permanent.
**Fetched by:** `/me/attendance` (calendar), the admin attendance report
(range-grouped counts per employee).

### `leave_types`, `leave_allocations`, `leave_requests`, `holidays`
Standard leave management: types with default yearly days, per-user-per-year
allocations, requests with an approval workflow
(`pending`/`approved`/`rejected`/`cancelled`, half-days supported), and the
company holiday calendar. **Fetched by:** the leave pages, and the attendance
derivation reads *approved* requests + holidays to explain non-worked days.

### `weekly_hours_reports`
One row per `(user, Mon–Sun week)`: `working_days` (business days minus
holidays/leave), `required_seconds` (days × 8h), `worked_seconds`,
`shortfall_seconds`, `compliant`, `notified_at` (one-time HR/PM warning
stamp). Written by the Monday-morning job from intervals + attendance.
Employee-only by design — managers/HR don't get shortfall warnings.

---

## 7. Candidate onboarding (pre-employee pipeline)

`onboarding_stages` (ordered Kanban columns: Applied → Interview → Offer →
Onboarding → Hired), `candidates` (with `converted_user_id` once hired),
`candidate_tasks` (checklist), `candidate_documents` (metadata only — bytes in
GCS). Currently not linked in the sidebar but the API/routes exist.

---

## Quick reference: which table answers which question

| Question | Table(s) | Path |
|---|---|---|
| Who is working right now? | `presence` + `users` | `GET /admin/team` |
| Hours today/this week? | `intervals` (SUM at read) | `/me/hours`, admin drill-down |
| What was on their screen? | `screenshots` → GCS presigned | day gallery |
| Are they working on their tickets? | `analysis_results`, `analysis_reports` | report card, nightly job |
| Were they present on the 3rd? | `attendance_days` | attendance calendar/report |
| Did they work full hours last week? | `weekly_hours_reports` | Monday job + report |
| Who approved that? | `audit_logs` | HR audit view |
