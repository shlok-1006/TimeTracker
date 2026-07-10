# TimeTracker

Cross-platform employee time-tracking platform: a desktop app that records work
intervals and screenshots, a hosted API with admin and employee web portals, and
AI-assisted screenshot analysis that scores activity against each employee's
assigned Linear tickets.

**Production:** <https://time-tracker.rapidinnovation.dev> · **Desktop installers:** [GitHub Releases](../../releases/latest)

## Features

- **Time tracking** — start/stop/break/meeting mode; idle detection; hours
  computed from immutable UTC intervals (never a mutable counter)
- **Local-first** — the desktop writes to SQLite before any network call and
  syncs in the background; a bad connection never loses data
- **Screenshots** — captured on a randomized cadence **only while working**
  (never in meetings or breaks), uploaded straight to Google Cloud Storage via
  short-lived signed URLs; the server stores metadata only
- **AI analysis** — Claude (vision) compares screenshots against the employee's
  open Linear tickets and manual tasks, producing per-screenshot verdicts, a
  daily alignment score, an AI summary, and low-score email alerts; admins can
  also run an exhaustive analysis over any day or time range with live progress
- **Attendance** — auto-marked present after 2 minutes of tracking; integrates
  approved leave, holidays, and weekends
- **Live presence** — the admin dashboard shows who is working / idle / on
  break / in a meeting right now
- **Leave & teams** — leave requests + HR approval, team selection, My Day
- **RBAC** — employees see only their own data; project managers see only their
  team; HR sees everything; all sensitive actions are audit-logged
- **Secure auth** — Argon2 password hashing, short-lived JWTs with rotating
  refresh tokens and reuse detection, tokens in the OS keychain, change-password
  at login, welcome emails with credentials

## Architecture

```
Desktop app (Tauri 2 / Rust / Next.js / SQLite)
      │  sync worker (intervals, presence, screenshot metadata)
      │  direct upload via signed URLs ────────────► Google Cloud Storage
      ▼
Cloudflare ─► Nginx ─┬─► API server (Rust / Axum / SQLx) ─► Supabase PostgreSQL
                     ├─► Admin dashboard  (Next.js, /)
                     └─► Employee portal  (Next.js, /employee)
```

| Component       | Technology                                     | Location            |
| --------------- | ---------------------------------------------- | ------------------- |
| Desktop app     | Tauri 2 + Rust + Next.js static export + SQLite | `apps/desktop`      |
| API server      | Rust + Axum + SQLx (compile-time checked SQL)  | `server`            |
| Admin dashboard | Next.js 15 + TypeScript + Tailwind             | `apps/admin-web`    |
| Employee portal | Next.js 15 + TypeScript + Tailwind             | `apps/employee-web` |
| Shared types    | Zod schemas / role enum                        | `packages/shared`   |
| Database        | Supabase PostgreSQL (prod) / local Postgres (dev) | —                |
| File storage    | Google Cloud Storage, V4-signed URLs           | —                   |

## Roles

| Role              | Access                                                        |
| ----------------- | ------------------------------------------------------------- |
| `employee`        | Desktop app + employee portal; own data only                  |
| `project_manager` | Admin dashboard; own team only                                |
| `hr`              | Admin dashboard; all employees, user management, audit logs   |

## Local development

Prerequisites: Rust (stable), Node ≥ 20, pnpm (version pinned by
`packageManager` in `package.json`), Docker, and the
[Tauri 2 system deps](https://tauri.app/start/prerequisites/).

```bash
cp .env.example .env          # fill in secrets

# Local Postgres + MinIO (the `local` profile is dev-only infra)
docker compose --profile local up -d postgres minio minio-init

pnpm install

# API server (applies migrations on startup)
cargo run -p server

# Admin dashboard → http://localhost:3001
pnpm --filter admin-web dev

# Desktop app
pnpm --filter desktop tauri dev
```

The server uses SQLx compile-time checked queries: either have `DATABASE_URL`
pointing at a migrated database when building, or build with `SQLX_OFFLINE=true`
(the committed `.sqlx/` cache). After adding or changing queries, refresh the
cache with `cd server && cargo sqlx prepare` and commit the result.

### Common commands

```bash
cargo test -p server                  # server tests
cargo test -p timetracker-desktop     # desktop crate tests
pnpm --filter admin-web typecheck     # frontend typecheck
cargo fmt && cargo clippy             # format + lint
pnpm --filter desktop tauri build     # desktop installer (local)
```

## Deployment

Production runs on a single VM with Docker Compose behind Nginx and Cloudflare.
The database is Supabase; screenshots live in a GCS bucket. Server images embed
their migrations and apply them on startup.

```bash
git pull origin main
docker compose build server admin-web employee-web
docker compose up -d --no-deps server admin-web employee-web
docker compose restart nginx
```

Key environment variables (see `.env.example` for the full list):

| Variable                | Purpose                                          |
| ----------------------- | ------------------------------------------------ |
| `DATABASE_URL`          | Postgres connection (Supabase session pooler)    |
| `GCS_SA_KEY_BASE64`     | Service-account key for GCS V4 URL signing       |
| `S3_BUCKET`             | Screenshot bucket name                           |
| `ANTHROPIC_API_KEY`     | Enables AI screenshot analysis (Claude)          |
| `LINEAR_API_KEY`        | Enables Linear ticket integration                |
| `SMTP_*`                | Welcome emails and low-score alerts              |

## Desktop releases

Releases are built by GitHub Actions (`.github/workflows/release.yml`): pushing
a `v*` tag builds installers for Windows, macOS (Intel + Apple Silicon), and
Linux, attached to a draft GitHub Release — publish it to go live. Release
builds have the production server URL baked in, so a fresh install needs no
configuration.

```bash
git tag v0.1.6 && git push origin v0.1.6
```

## Documentation

- [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) — the original step-by-step build
  log with per-module implementation notes and verification commands
- `CLAUDE.md` — architecture rules and coding standards enforced across the repo
- `TimeTracker-*.pdf` — deployment, security, and launch documents

---

Internal tool of RUH / Rapid Innovation.
