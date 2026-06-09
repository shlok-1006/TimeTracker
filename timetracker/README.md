# TimeTracker — STEP 0: Project Foundations

Cross-platform employee time tracking platform. This step establishes the
monorepo, build systems, local infrastructure, and a working `GET /health`
endpoint. Feature modules (timer, idle, sync, auth, RBAC, screenshots) are
layered on in subsequent steps.

## Stack

| Component       | Technology                                            |
| --------------- | ----------------------------------------------------- |
| Desktop         | Tauri 2 + Rust + Next.js 15 (static export) + SQLite  |
| Admin Dashboard | Next.js 15 + TypeScript + Tailwind + Shadcn UI        |
| API Server      | Rust + Axum + SQLx                                     |
| Database        | PostgreSQL                                            |
| Object Storage  | MinIO (local) / Cloudflare R2 (production)            |

## File tree

```
timetracker/
├── Cargo.toml                  # Rust workspace (server + desktop)
├── package.json                # pnpm workspace root + scripts
├── pnpm-workspace.yaml
├── docker-compose.yml          # PostgreSQL + MinIO
├── rust-toolchain.toml
├── .env.example                # copy to .env
├── apps/
│   ├── desktop/                # Tauri 2 app
│   │   ├── package.json        # Next.js (static export) frontend
│   │   ├── next.config.mjs     # output: "export"
│   │   ├── src/app/            # layout.tsx, page.tsx, globals.css
│   │   └── src-tauri/          # Rust shell
│   │       ├── Cargo.toml
│   │       ├── tauri.conf.json
│   │       ├── build.rs
│   │       ├── capabilities/default.json
│   │       ├── icons/          # generated app icons
│   │       └── src/            # main.rs + lib.rs (run + app_info command)
│   └── admin-web/              # Next.js 15 admin dashboard
│       ├── package.json
│       ├── components.json     # Shadcn config
│       ├── tailwind.config.ts
│       └── src/app/            # layout.tsx, page.tsx (calls /health), globals.css
├── server/                     # Axum API
│   ├── Cargo.toml
│   ├── migrations/0001_init.sql
│   ├── src/
│   │   ├── main.rs             # boot: config → db → migrate → serve
│   │   ├── lib.rs              # public crate surface
│   │   ├── config.rs
│   │   ├── error.rs            # AppError : IntoResponse
│   │   ├── state.rs            # AppState { db: PgPool }
│   │   ├── db/mod.rs           # connect + run_migrations
│   │   └── routes/
│   │       ├── mod.rs          # router + CORS + tracing
│   │       └── health.rs       # GET /health, GET /ready
│   └── tests/health.rs         # integration test
└── packages/
    └── shared/                 # @timetracker/shared (roles, API types, Zod)
```

## Prerequisites

- Rust (stable) + Cargo — <https://rustup.rs>
- Node.js ≥ 20 and pnpm ≥ 9 — `npm install -g pnpm`
- Docker (for PostgreSQL + MinIO)
- Tauri 2 system deps — <https://tauri.app/start/prerequisites/>
- `sqlx-cli` (optional, for manual migrations) — `cargo install sqlx-cli --no-default-features --features rustls,postgres`

## Setup & run

```bash
# 0. From the timetracker/ directory, create your env file
cp .env.example .env

# 1. Start infrastructure (PostgreSQL + MinIO + bucket)
docker compose up -d

# 2. Install JS dependencies (whole workspace)
pnpm install

# 3. Run the API server (auto-applies migrations on startup)
cargo run -p server
#    -> listening on http://localhost:8090
#    -> GET http://localhost:8090/health  => {"status":"ok"}

# 4. Run the admin dashboard (separate terminal)
pnpm --filter admin-web dev
#    -> http://localhost:3001  (shows live API health)

# 5. Run the desktop app (separate terminal)
pnpm --filter desktop tauri dev
#    -> launches the Tauri window; Next.js dev server on :3000
```

### Manual migrations (optional)

The server applies migrations automatically. To run them by hand:

```bash
cd server
sqlx migrate run            # uses DATABASE_URL from ../.env or your shell
```

## Verify acceptance criteria

```bash
# Server up + health returns 200
curl -i http://localhost:8090/health        # HTTP/1.1 200 OK, {"status":"ok"}
curl -i http://localhost:8090/ready         # 200 once Postgres is reachable

# Server tests (includes the /health integration test)
cargo test -p server

# Lint / format
cargo fmt --check && cargo clippy --workspace
pnpm -r lint
```

## Ports

| Service          | Port |
| ---------------- | ---- |
| API server       | 8090 |
| Desktop frontend | 3000 |
| Admin dashboard  | 3001 |
| PostgreSQL       | 5432 |
| MinIO S3 API     | 9000 |
| MinIO console    | 9001 |

## Architecture notes

- **Workspaces.** One Cargo workspace (`server`, `apps/desktop/src-tauri`) pins
  shared Rust dependency versions in `[workspace.dependencies]`. One pnpm
  workspace (`apps/*`, `packages/*`) shares the `@timetracker/shared` package,
  which is the single source of truth for the role enum and API contracts — no
  magic strings duplicated across the stack (mirrors the Rust `UserRole` enum and
  Postgres `user_role` type).
- **Server boot is fail-fast.** `main.rs` reads config, connects to Postgres,
  runs embedded migrations, then serves. Any failure aborts startup with a
  propagated error (Rule 8 — no `unwrap` in production paths). `AppError`
  implements `IntoResponse` so handlers return `Result<_, AppError>`.
- **`/health` is dependency-free** (liveness); `/ready` pings the DB (readiness).
- **UTC everywhere** (Rule 3): the schema uses `TIMESTAMPTZ` and `now()`.
- **Audit logs are immutable** (DB trigger blocks UPDATE/DELETE).
- **Desktop is local-first** (Rule 1): the Tauri shell owns a SQLite source of
  truth; it never talks to Postgres directly (Rule 4). STEP 0 ships the shell and
  a proven Rust⇄JS bridge (`app_info` command); the SQLite layer and sync worker
  arrive in later steps.
- **Screenshots** (Rule 5): the server stores only metadata; bytes live in
  MinIO/R2. The `screenshots` bucket is provisioned by `docker compose`.

---

## STEP 1 — Role-Based Authentication

Roles (`user_role` enum): `employee`, `project_manager`, `hr`.

- **Employee** → desktop app only.
- **HR / project manager** → admin dashboard only.

### Backend
- `POST /auth/login` — email + password → JWT. Payload: `{ sub, role, team, exp }` (HS256).
- Argon2id password hashing (`auth.rs`); JWT issue/verify (`jwt.rs`); bearer-token
  middleware + role guards `require_employee` / `require_admin` (HR or PM) /
  `require_hr` (`middleware.rs`).
- Protected demo endpoints prove the guards: `GET /me` (any auth),
  `/desktop/ping` (employee), `/dashboard/ping` (HR/PM), `/hr/ping` (HR).
  A token with the wrong role gets **403**.

> **Reconciliation note:** STEP 1 redefines the roles from STEP 0
> (`employee/manager/admin`). The `0001_init.sql` enum was updated accordingly.
> The requested guard `require_admin()` maps to the admin-dashboard roles
> (`hr` + `project_manager`), since there is no standalone `admin` role.

### Clients
- **Desktop**: login screen → Tauri `login` command calls the API, **accepts
  `employee` only**, and stores the JWT in the **OS keychain** (`keyring` crate).
- **Admin dashboard**: login screen → calls the API, **accepts `hr` /
  `project_manager` only**; token kept in a Zustand store.

### Seed data
Canonical seed (idempotent):
```bash
cargo run -p server --bin seed
```
Creates:

| Role     | Email                        | Password        |
| -------- | ---------------------------- | --------------- |
| HR       | `hr@timetracker.local`       | `ChangeMe!HR1`  |
| Employee | `employee@timetracker.local` | `ChangeMe!Emp1` |

### Build/run notes
- The server uses **compile-time checked queries** (Rule 7), so **Postgres must
  be running and `DATABASE_URL` set when you build the server** (`cargo build`/
  `run`/`test` for the `server` crate connect to verify SQL). Build from the
  `timetracker/` dir so the root `.env` is picked up, or export `DATABASE_URL`.
- Tests: `cargo test -p server` (role guards → 401/403, Argon2 hash/verify, JWT
  round-trip; the guard tests need no DB).

### Verify
```bash
TOKEN=$(curl -s localhost:8090/auth/login -H 'content-type: application/json' \
  -d '{"email":"hr@timetracker.local","password":"ChangeMe!HR1"}' | jq -r .access_token)
curl -i localhost:8090/dashboard/ping -H "Authorization: Bearer $TOKEN"   # 200 (HR)
curl -i localhost:8090/desktop/ping  -H "Authorization: Bearer $TOKEN"    # 403 (wrong role)
```

---

## STEP 2 — Time Recording Engine

Local-first interval tracking (Rules 1–4): the desktop records immutable
intervals to SQLite, a background worker syncs them to the API, and totals are
always computed from intervals (never a stored counter).

### Desktop (`apps/desktop/src-tauri/src/`)
- `db.rs` — local SQLite pool + migrations (`migrations/0001_intervals.sql`).
- `interval_repository.rs` — append-only `intervals` + a separate
  `interval_sync` queue (keeps intervals immutable); totals via `sum_worked`.
- `timer.rs` — `start_tracking` / `stop_tracking`. Duration comes from a
  **monotonic clock** (`Instant`), so wall-clock jumps can't corrupt it;
  `end_utc = start_utc + elapsed`. Commands: `start_tracking`, `stop_tracking`,
  `is_tracking`, `get_total_seconds`.
- `sync_worker.rs` — every 15s, pushes pending intervals to `POST /intervals`
  with the stored JWT; marks them synced on success. Non-blocking, at-least-once.

The local DB lives in the OS app-data dir (`…/com.timetracker.desktop/timetracker.db`),
so intervals survive restarts.

### Server
- `migrations/0002_intervals.sql` — `intervals` table (immutable via trigger,
  UTC, FK to users, ordered-check).
- `POST /intervals` — batch sync; `user_id` taken from the JWT (never the body);
  idempotent (`ON CONFLICT (id) DO NOTHING`).
- `GET /me/hours` — total worked seconds, computed in SQL from intervals.

### Tests
- Desktop (`cargo test -p timetracker-desktop`): monotonic finalize, totals
  exclude idle, insert/totals round-trip, **survives restart** (file DB reopen),
  sync-queue flow — 5 passing.
- Server (`cargo test -p server`): `/intervals` and `/me/hours` require auth.

### Verify end-to-end
```bash
TOKEN=$(curl -s localhost:8090/auth/login -H 'content-type: application/json' \
  -d '{"email":"employee@timetracker.local","password":"ChangeMe!Emp1"}' | jq -r .access_token)
# sync an interval
curl -s -X POST localhost:8090/intervals -H "Authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '[{"id":"11111111-1111-1111-1111-111111111111","start_utc":"2026-06-02T09:00:00Z","end_utc":"2026-06-02T10:00:00Z","idle":false}]'
# totals computed from intervals
curl -s localhost:8090/me/hours -H "Authorization: Bearer $TOKEN"   # {"total_seconds":3600,...}
```
Or just run the desktop app: **Start tracking → wait → Stop**; the interval
persists locally and syncs within ~15s.

> The server gained new routes in STEP 2 — **restart `cargo run -p server`** to pick them up.

---

## STEP 3 — Presence & Live Status

Statuses: `working`, `idle`, `break`, `not_logged_in`.

### Desktop (`apps/desktop/src-tauri/src/`)
- `idle.rs` — idle detection via the **`device_query`** crate. A background
  thread samples mouse + keys every 2s; `is_idle` true after no activity for the
  configurable threshold (`TIMETRACKER_IDLE_THRESHOLD_SECS`, default 60s).
- `presence.rs` — `derive_status(on_break, is_idle)` →
  `break` > `idle` > `working`; the **heartbeat worker POSTs `/presence` every
  45s** while logged in. Commands: `set_break`, `is_on_break`, `current_status`.
- Transitions: working↔idle (automatic), working↔break (manual Break button).

### Server
- `migrations/0003_presence.sql` — `presence` table
  (`user_id, status, last_seen_at, current_interval_id`) + `presence_status` enum.
- `POST /presence` — heartbeat; `user_id` from JWT; rejects `not_logged_in`.
- `GET /admin/team` (HR / project-manager) — live roster. Status is
  **derived server-side**: no row or `last_seen_at` older than the 90s grace
  period ⇒ `not_logged_in`. HR sees all employees; a PM sees their own team.

### Admin dashboard
After login, a **live team table** polls `/admin/team` every 10s (TanStack Query)
and shows each employee's status + "last seen".

### Tests
- Desktop: idle threshold + reset, status transitions (incl. break-overrides-idle).
- Server: presence/admin routes require auth; `/admin/team` is 403 for employees.

### Verify
```bash
EMP=$(curl -s localhost:8090/auth/login -H 'content-type: application/json' \
  -d '{"email":"employee@timetracker.local","password":"ChangeMe!Emp1"}' | jq -r .access_token)
HR=$(curl -s localhost:8090/auth/login -H 'content-type: application/json' \
  -d '{"email":"hr@timetracker.local","password":"ChangeMe!HR1"}' | jq -r .access_token)
curl -s -X POST localhost:8090/presence -H "Authorization: Bearer $EMP" \
  -H 'content-type: application/json' -d '{"status":"working"}'
curl -s localhost:8090/admin/team -H "Authorization: Bearer $HR"   # employee => working
```
Or run both apps: the desktop sends heartbeats automatically; toggle **Break**
and watch the admin dashboard update within ~10s. Close the desktop app and the
employee flips to **not logged in** after ~90s.

> The server gained new routes in STEP 3 — **restart `cargo run -p server`**.

---

## STEP 4 — Screenshot Capture & Upload

The server never stores image bytes (Rule 5): it issues short-lived presigned
PUT URLs, the desktop uploads directly to storage, then posts metadata only.

### Desktop (`apps/desktop/src-tauri/src/screenshot.rs`)
- Captures the primary monitor with **`xcap`**, encodes JPEG (`image`).
- Runs on a configurable interval (`TIMETRACKER_SCREENSHOT_INTERVAL_SECS`,
  default 300s) and **only while `working`** — never on break, idle, meeting, or
  when not tracking.
- Flow: `POST /uploads/presign` → `PUT` bytes to storage → `POST /screenshots`.

### Server
- `storage.rs` — S3-compatible **SigV4 presigner** (pure Rust, verified against
  AWS's documented test vector; path-style for MinIO).
- `upload_service.rs` — mints presigned PUTs with a user-namespaced key
  (`<user_id>/<yyyymmdd>/<uuid>.jpg`).
- `migrations/0005_screenshots.sql` + `db/screenshots.rs` — metadata table
  (`id, user_id, storage_key, taken_at, interval_id`).
- `POST /uploads/presign` → `{ url, method, storage_key, expires_in }`.
- `POST /screenshots` → stores metadata; rejects keys outside the caller's
  namespace.

### Storage config (`.env`)
`S3_ENDPOINT`, `S3_REGION`, `S3_BUCKET`, `S3_ACCESS_KEY_ID`,
`S3_SECRET_ACCESS_KEY`, `S3_FORCE_PATH_STYLE` (defaults target local MinIO).

### Running storage without Docker
Use the standalone MinIO binary:
```powershell
# download minio.exe, then:
$env:MINIO_ROOT_USER='minioadmin'; $env:MINIO_ROOT_PASSWORD='minioadmin'
.\minio.exe server C:\minio-data --console-address ":9001"
# create the bucket (mc.exe) or via the console at http://localhost:9001
mc alias set local http://localhost:9000 minioadmin minioadmin
mc mb local/screenshots
```
For production, point the `S3_*` vars at a Cloudflare R2 bucket (set
`S3_FORCE_PATH_STYLE=false`).

### Tests
- Server: `matches_aws_documented_vector` (SigV4 correctness), path-style
  presign, namespaced key.
- Desktop: `captures_only_while_working`, JPEG encoding.

> New routes/enum in STEP 4 — **restart `cargo run -p server`**. Screenshots
> upload to storage only once MinIO/R2 is reachable; presign + metadata work
> regardless.

---

## STEP 5 — Employee Dashboard (desktop)

The desktop UI is now a full dashboard. **Local-first:** hours/charts render
from local SQLite instantly, then reconcile with the server.

### Endpoints
- `GET /me/hours` — enriched summary `{ total, today, week, active, idle }`,
  computed from intervals (Rule 2). Scoped to the caller (token `sub`).
- `GET /me/screenshots` — own screenshots with short-lived **presigned view
  URLs** (Rule 5; raw keys never exposed). Scoped to the caller.

### Desktop (`apps/desktop/src-tauri/src/`)
- `reports.rs` — `summarize` + `daily_timeline` over local SQLite intervals;
  commands `get_hours_summary`, `get_daily_timeline`.
- `client.rs` — authenticated proxy commands `me_hours`, `me_screenshots` (the
  JWT stays in the keychain; the webview never sees it).

### UI (`apps/desktop/src/app/page.tsx`) — React + Tailwind + TanStack Query + recharts
- Cards: **Today's Hours**, **This Week**, **Current Status**.
- Charts: **Active vs Idle** (donut), **Daily Timeline** (7-day bars).
- **Screenshot gallery**: thumbnail strip + click-to-zoom modal.
- A reconcile line shows the server total once `/me/hours` resolves.

### Access
Everything is `/me/*` (data derived from the JWT subject) — an employee sees
only their own hours/screenshots/status and cannot reach `/admin/*` (403).

### Tests
- Desktop: `summarize_today_week_total_idle`, `daily_timeline_has_seven_buckets`.
- Server: `/me/hours` SQL validated; `/me/screenshots` presigns GET URLs.

> Screenshots only display once MinIO/R2 is running (the gallery hides images
> that fail to load). **Restart `cargo run -p server`** for the new routes.

---

## STEP 6 — Admin Dashboard

For `hr` and `project_manager`. Live status board + per-employee drill-down.

### Endpoints
- `GET /admin/team` — roster with **Name, Status, Last seen, Today's hours**
  (today's hours computed per user from intervals). Scope: HR → all employees;
  PM → only their team (`users.manager_id = <pm>`).
- `GET /admin/users/:id/hours` — drill-down hours summary.
- `GET /admin/users/:id/screenshots` — drill-down screenshots (presigned URLs).
- All three require HR/PM (employees → 403). Drill-downs enforce scope: a PM
  viewing a non-team user → **403**; HR → any user.

### Dashboard (admin-web)
- Employee table, **auto-refreshing every 30s** (TanStack Query).
- Status colors: **green** Working · **yellow** Idle · **blue** Break ·
  **red** Not logged in (plus purple Meeting, slate Not working).
- **Row click → drill-down modal**: hours (today/week/active/idle) + screenshot
  gallery with click-to-zoom.

### Permissions
- **HR** sees everyone. **PM** sees only their assigned team — enforced both in
  the team query and on every drill-down (`authorize_view`).

### Tests
- Server: drill-downs require auth (401) and reject employees (403).
- Live e2e (verification flow): HR sees all; PM sees own team only (non-team
  user → 403).

> Assign a team by setting `users.manager_id = <pm-id>` for an employee.
> **Restart `cargo run -p server`** for the new routes.

---

## STEP 7 — MVP Hardening

### Secure token management (Rule 6)
- **Access token** — short-lived JWT (default 15 min, `JWT_ACCESS_TTL_SECONDS`).
- **Refresh token** — opaque random string; only its **SHA-256 hash** is stored
  (`refresh_tokens` table — no plaintext at rest). `POST /auth/refresh` rotates
  it (the used token is revoked, a new pair issued). `POST /auth/logout` revokes.
- **Desktop**: both tokens live in the **OS keychain** (`keyring`,
  `windows-native`/`apple-native`/`secret-service`). `http.rs` adds the bearer
  and, on `401`, **transparently refreshes once and retries** — so all workers
  (sync, presence, screenshot) and the dashboard keep working for long sessions.
- Endpoints: `POST /auth/login` → `{access_token, refresh_token, expires_in}`;
  `POST /auth/refresh`; `POST /auth/logout`.

### Recording notice (transparency)
The desktop shows an **always-visible "Screen recording active" indicator**
whenever screenshots are being captured (while Working), and a red banner if the
OS hasn't granted screen-recording permission.

### Permissions
- A `check_capture` command probes whether screen capture works; the UI warns if
  it doesn't (covers macOS/Wayland where the user must grant access).
- **macOS**: grant *System Settings → Privacy & Security → Screen Recording →
  TimeTracker*. (Screen recording is TCC-gated; first capture prompts.)
- **Linux/Wayland**: capture goes through the desktop portal — install
  `xdg-desktop-portal` (+ `xdg-desktop-portal-wlr`/`-gnome`) and `pipewire`.
  X11 sessions work out of the box.

### CI/CD (`.github/workflows/`)
- **`ci.yml`** — on push/PR: server lint+test (against a Postgres service,
  migrations applied via `psql`), desktop-crate build+test (Linux Tauri deps),
  and frontend typecheck+build.
- **`release.yml`** — on tag `v*` (or manual): `tauri-action` matrix builds
  installers for **Windows** (`.msi`/`.exe`), **macOS** (`.dmg`, Intel + Apple
  Silicon), and **Linux** (`.deb`/`.AppImage`), attached to a draft release.

### Tests
- Server: refresh-token uniqueness + hash stability (no plaintext); existing
  suites unchanged.
- Live e2e (verification): login → refresh rotates → old refresh token rejected.

> The server gained `/auth/refresh` + `/auth/logout` — **restart
> `cargo run -p server`**, then sign out/in on the desktop so both tokens are
> stored. Sessions now survive access-token expiry via silent refresh.

---

## STEP 8 — Linear Integration (read-only)

Links employees to Linear and serves their assigned tickets. The Linear API
token is **server-side only** (`LINEAR_API_KEY`) — never sent to any client.

### Files (`server/src/`)
- `linear_service.rs` — GraphQL client + `link_user_to_linear`,
  `fetch_assigned_tickets` (hourly cache + stale fallback on rate-limit),
  `get_ticket_context`.
- `ticket_cache.rs` — per-user in-memory cache (1h TTL) that serves stale
  results when a live fetch is rate-limited/fails.
- `db/linear_repository.rs` + `migrations/0009_linear_links.sql` — `linear_links`
  table (`user_id`, `linear_user_id`, `linked_at`). The token is **not** stored.

### Endpoints (all require auth; employee-scoped)
- `POST /me/linear/link` — match the caller's email to a Linear user and store
  the link. → `{ linked, linear_user_id }`.
- `GET /me/tickets` → `{ tickets: [{ id, title, state, project, labels,
  description_excerpt }] }`.
- `GET /me/tickets/:id/context` — full ticket context.

### Setup
1. Linear → **Settings → Security & access → Personal API keys** → create one.
2. Put it in `.env`: `LINEAR_API_KEY=lin_api_...` and restart the server.
3. Ensure the employee's email matches their Linear account email.
4. `POST /me/linear/link` once, then `GET /me/tickets`.

If `LINEAR_API_KEY` is empty the endpoints return a clear "not configured" error.

### Requirements met
- **Token never exposed** to clients (server-held; only ticket data is returned).
- **Cached** hourly; **rate limits** handled (429 → serve stale cache).
- **Read-only** (only GraphQL queries, no mutations).
- Tests: GraphQL parsing, excerpt truncation, cache TTL/stale, route auth (401).

### Verify
```bash
TOKEN=$(curl -s localhost:8090/auth/login -H 'content-type: application/json' \
  -d '{"email":"employee@timetracker.local","password":"ChangeMe!Emp1"}' | jq -r .access_token)
curl -s -X POST localhost:8090/me/linear/link -H "Authorization: Bearer $TOKEN"   # links by email
curl -s localhost:8090/me/tickets -H "Authorization: Bearer $TOKEN" | jq          # assigned tickets
```

> **Restart `cargo run -p server`** after setting `LINEAR_API_KEY`.
