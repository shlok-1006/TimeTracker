# CLAUDE.md

## Project

Cross-platform employee time tracking platform.

Components:

1. Desktop App

   * Tauri 2
   * Rust
   * Next.js 15 Static Export
   * SQLite

2. API Server

   * Rust
   * Axum
   * PostgreSQL
   * SQLx

3. Admin Dashboard

   * Next.js 15
   * TypeScript
   * Shadcn UI

4. Storage

   * Cloudflare R2 (S3 compatible)

---

## Architecture Rules

### Rule 1: Local First

The desktop application is always the source of truth for raw activity.

All data must be written to SQLite before any network operation.

Never block user actions waiting for API responses.

---

### Rule 2: Time Tracking

Time is represented as immutable intervals.

Never maintain a mutable "total hours" counter.

Correct:

Interval:

* start_utc
* end_utc
* idle

Totals are calculated from intervals.

Incorrect:

daily_hours = daily_hours + 1

---

### Rule 3: UTC Everywhere

All timestamps stored in databases must be UTC.

Timezone conversion happens only in the UI layer.

---

### Rule 4: Sync Architecture

SQLite → Sync Queue → API → PostgreSQL

The desktop app never writes directly to PostgreSQL.

All network synchronization must go through the sync worker.

---

### Rule 5: Screenshots

The server never stores screenshot bytes.

The server stores:

* screenshot_id
* user_id
* storage_key
* thumbnail_key
* timestamp

Actual files live in Cloudflare R2.

Upload flow:

Desktop
→ Request presigned URL
→ Upload directly to R2
→ Notify API
→ Save metadata

---

### Rule 6: Authentication

Authentication uses:

* JWT Access Token
* Refresh Token Rotation
* Argon2 Password Hashing

Tokens must be stored using OS Keychain via keyring crate.

Never store access tokens in plaintext files.

---

### Rule 7: Database Access

Use SQLx only.

Requirements:

* Compile-time checked queries
* Repository pattern
* Migrations for every schema change

Avoid ORM frameworks.

---

### Rule 8: Error Handling

Use:

* anyhow
* thiserror

Never use unwrap() in production code.

All errors must be propagated properly.

---

### Rule 9: Testing

Every feature should include:

* Unit Tests
* Integration Tests

Critical modules:

* timer.rs
* idle.rs
* sync.rs
* auth.rs

Must maintain test coverage above 80%.

---

### Rule 10: Frontend

Use:

* React
* TypeScript
* Tailwind
* Shadcn UI
* TanStack Query
* Zustand

Avoid Redux.

---

## Rule 11: Role-Based Access Control (RBAC)

The system supports three roles:

### Employee

Permissions:

* Start tracking
* Stop tracking
* Pause tracking
* Resume tracking
* View own hours
* View own screenshots
* View own activity logs
* View own productivity reports
* Update own profile
* Manage personal settings

Restrictions:

* Cannot view other employees
* Cannot view team analytics
* Cannot view company reports
* Cannot access admin endpoints
* Cannot manage users

---

### Manager

Permissions:

Everything Employee can do, plus:

* View team members
* View team hours
* View team productivity
* View team screenshots
* View team activity logs
* View attendance reports
* View AI summaries for team
* Export team reports

Restrictions:

* Cannot modify system settings
* Cannot create admins
* Cannot delete users
* Cannot access other manager teams unless explicitly assigned

Managers should only have access to employees assigned to their team.

---

### Admin

Permissions:

Full system access.

Can:

* Manage all users
* Create users
* Update users
* Delete users
* Assign managers
* Assign teams
* View all screenshots
* View all productivity data
* Access audit logs
* Manage storage settings
* Manage screenshot policies
* Configure retention rules
* Configure tracking policies
* Access system analytics

---

## Database Schema

users

id UUID PRIMARY KEY
name TEXT
email TEXT UNIQUE
password_hash TEXT
role TEXT
manager_id UUID NULL
team_id UUID NULL

Allowed roles:

* employee
* manager
* admin

Never use magic strings in code.

Create Role enum:

enum UserRole {
Employee,
Manager,
Admin
}

---

## Authorization Rules

Every protected endpoint must validate:

1. Authentication
2. Authorization

Example:

Employee:
GET /me/hours
Allowed

Employee:
GET /team/hours
Denied

Manager:
GET /team/hours
Allowed

Manager:
GET /admin/users
Denied

Admin:
GET /admin/users
Allowed

---

## Backend Architecture

Use middleware:

auth_middleware.rs

Responsibilities:

* Validate JWT
* Extract user
* Attach role to request context

Create authorization guards:

require_employee()
require_manager()
require_admin()

Usage:

Only authenticated users:

* employee+
* manager+
* admin

Manager-only:

* manager
* admin

Admin-only:

* admin

---

## Frontend Authorization

Never rely only on frontend role checks.

Frontend checks are for UX.

Backend checks are mandatory.

Use role-based route protection.

Examples:

Employee:

* /dashboard
* /profile
* /settings

Manager:

* /dashboard
* /team
* /reports

Admin:

* /dashboard
* /team
* /reports
* /users
* /audit
* /settings/system

---

## Screenshot Access Policy

Employee:
Can view only own screenshots.

Manager:
Can view screenshots of employees assigned to their team.

Admin:
Can view all screenshots.

All screenshot URLs must be generated through short-lived presigned URLs.

Never expose storage keys directly.

---

## Audit Logging

Record all sensitive actions.

Examples:

* User login
* User logout
* User creation
* User deletion
* Role change
* Screenshot access
* Report export
* Settings modification

Audit table:

audit_logs

id UUID
actor_id UUID
action TEXT
entity_type TEXT
entity_id UUID
created_at TIMESTAMP UTC

Audit logs are immutable.

Only admins may view audit logs.


## Folder Structure

timetracker/

apps/
├── desktop/
├── admin-web/

server/

packages/
└── shared/

---

## Desktop Modules

src-tauri/src/

main.rs

timer.rs

* interval engine

idle.rs

* activity detection

screenshot.rs

* screenshot capture

sync.rs

* synchronization

activity_tracker.rs

* active applications

productivity.rs

* productivity scoring

db.rs

* SQLite access

auth.rs

* local auth handling

---

## API Modules

server/src/

main.rs

routes/

* auth
* users
* hours
* uploads
* screenshots
* teams

services/

* auth_service
* upload_service
* reporting_service

db/

* repositories
* migrations

---

## Coding Standards

Rust:

* Prefer structs over global state.
* Use dependency injection.
* Keep functions small and testable.
* Use async where appropriate.
* Avoid unnecessary cloning.

TypeScript:

* Strict mode enabled.
* No any types.
* Use Zod validation.
* Use reusable hooks.

---

## Performance Targets

Desktop:

* Startup under 2 seconds
* Memory under 200 MB
* CPU under 3% while idle

Server:

* API response under 300ms
* Support 10,000+ users

---

## Security Requirements

Implement:

* RBAC
* Rate Limiting
* Audit Logs
* TLS
* Encrypted Storage
* Secure Token Handling

Never expose screenshot URLs publicly.

Always use short-lived presigned URLs.

---

## Commands

Run Desktop:

pnpm --filter desktop tauri dev

Build Desktop:

pnpm --filter desktop tauri build

Run Server:

cargo run -p server

Database Migration:

sqlx migrate run

Format:

cargo fmt

Lint:

cargo clippy

pnpm lint

Test:

cargo test

pnpm test

---

## Development Workflow

When implementing a feature:

1. Design database schema first.
2. Create migrations.
3. Create repositories.
4. Create services.
5. Create API routes.
6. Create frontend integration.
7. Add tests.
8. Update documentation.

Always provide:

* File tree changes
* Migration files
* Tests
* Explanation of architecture decisions

Do not generate pseudocode.

Generate production-ready code only.
