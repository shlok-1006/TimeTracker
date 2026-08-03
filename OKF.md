# OKF — Company Rulebook (Single Source of Truth for Policy)

> **What this file is.** The OKF is the one authoritative, **HR-editable** list of every
> company policy rule TimeTracker enforces — leave, attendance, working-hours, screenshots,
> reminders, roles, security. HR edits the **Value** columns here; a reconciliation agent
> reads this file, compares each rule to what the running system actually does (the
> **System binding**), and — when they disagree — updates the system to match the OKF (or
> flags the mismatch for a human when it can't safely auto-apply).
>
> **Companion docs:** [`CLAUDE.md`](CLAUDE.md) (coding rules), [`PRD.md`](PRD.md) (product/ops),
> [`OKR_PRD.md`](OKR_PRD.md) (OKR module). Where the OKF and those disagree, **the OKF wins for
> policy values** — see §12 for the current known drifts.
>
> **Status:** v1.0 · Last reviewed 2026-07-27 · Org time zone **Asia/Kolkata** · Storage is UTC.

---

## 1. How the OKF works (the reconciliation contract)

Every rule below has a stable **ID**, a **Value** HR may change, and a **Change type** that
tells the agent *how* a mismatch is fixed:

| Change type | Where the value really lives | How the agent reconciles a mismatch |
|---|---|---|
| **`env`** | An environment variable read at server/desktop startup (a code default applies if unset). | Update the variable on the VM `.env` (or the code default), then restart the affected service. |
| **`db`** | A row HR maintains at runtime (e.g. leave types). | Apply through the existing HR admin API — never by raw SQL. |
| **`code`** | A hard-coded constant in a source file. | Open a pull request editing the cited constant; **do not hot-patch production**. Ships on the next release/deploy. |

**Agent rules of engagement (mandatory):**

1. The agent reconciles **policy only** — it never reads, moves, or deletes employee data.
2. Every change the agent makes is written to `audit_logs` (action `okf.reconcile`, before/after value).
3. `code`-type changes are proposed as a PR for human review; `env`/`db` changes may be applied
   directly **only** inside an approved maintenance action, otherwise proposed.
4. If a rule's Value is ambiguous, out of its allowed range, or the binding no longer exists,
   the agent **stops and reports** — it does not guess.
5. The **System binding** (`file:line`) is the contract. If code moves, update the binding here
   in the same change so the OKF stays trustworthy.

**Value formats:** durations may be written as `4h`, `30m`, `45s`, `30d` or raw seconds; the
agent normalises to the unit the binding expects. `—` means "no limit / not set".

---

## 2. Attendance

| ID | Rule | Value (HR-editable) | Change type | System binding |
|---|---|---|---|---|
| ATT-01 | **Half-day (partial) threshold.** A *completed* day with tracked time **below** this is marked `partial`; at/above it, `present`. | **4h** (14400 s) | `env` `TIMETRACKER_ATTENDANCE_FULL_DAY_SECONDS` | `server/src/attendance_service.rs:22` |
| ATT-02 | **Auto-mark present.** Attendance flips to `present` the instant tracking starts (any heartbeat while working/idle/meeting/break) — no attendance-page visit, no wait. | **On** | `code` | `server/src/routes/presence.rs:44`, `attendance_service.rs:225` |
| ATT-03 | **Allowed statuses.** The complete set a day can hold. | `present · partial · absent · leave · holiday · weekend` | `code` | `server/migrations/0034_attendance_partial.sql:6` |
| ATT-04 | **Status precedence** when no time was tracked. | `leave → holiday → weekend → absent` | `code` | `server/src/attendance_service.rs:63` |
| ATT-05 | **Business-day boundary.** When "a day" starts for attendance rollup. ⚠️ See §12-C: this is **UTC midnight** today, while the dashboard uses 4 AM local. | UTC midnight *(target: 4 AM Asia/Kolkata)* | `code` | `server/src/attendance_service.rs:34` |
| ATT-06 | **Weekend days.** Days counted as `weekend`. | Sat, Sun | `code` | `server/src/leave_service.rs:12` |
| ATT-07 | **Weekends never count as a work day.** A Sat/Sun stays `weekend` even when the employee tracks time — it is never `present` or `partial`. Auto-present on Start is also skipped on weekends. | Weekends = `weekend` always | `code` | `server/src/attendance_service.rs:47` (`derive_status`), `:225` (`mark_present_today`) |

---

## 3. Leave

| ID | Rule | Value (HR-editable) | Change type | System binding |
|---|---|---|---|---|
| LV-01 | **Leave categories exist as HR-defined types** — there are no fixed built-in categories. HR creates/edits/deletes them. | *HR-managed at runtime* | `db` | `server/src/routes/leave.rs:198` |
| LV-02 | **Default annual allotment — employee** (also applies to PM & HR, who are treated as the employee category). Per leave type. | per-type `default_days` (seed **0**) | `db` | `server/migrations/0032_leave_category_defaults.sql:6`; `server/src/db/leave.rs:298` |
| LV-03 | **Default annual allotment — contractor.** Per leave type. | per-type `default_days_contractor` (seed **0**) | `db` | `server/src/db/leave.rs:298` |
| LV-04 | **Default annual allotment — intern.** Per leave type. | per-type `default_days_intern` (seed **0**) | `db` | `server/src/db/leave.rs:298` |
| LV-05 | **Manual HR adjustment.** HR may increase/decrease a person's balance by a delta; result **clamped to ≥ 0**. | Enabled; floor 0 | `db` (+ `code` for the floor) | `server/src/db/leave.rs:337`; route `routes/leave.rs:314` |
| LV-06 | **Who may configure leave.** Types/allocations/holidays. | **HR only** (PM may only approve their own team's requests) | `code` | `server/src/routes/leave.rs:106` |
| LV-07 | **Balance model.** Per **calendar year**; `remaining = allotted − approved-used`. **No accrual, no reset job. Carry-over: none today — target is LV-11 (≤ 10 days/yr).** | Carry-over per LV-11 (target) | `code` | `server/src/db/leave.rs:375` |
| LV-08 | **Day counting.** Leave excludes weekends + holidays; **half-days supported** (0.5). Requests are rejected if `remaining < requested`. | Excl. Sat/Sun + holidays; 0.5 allowed | `code` | `server/src/leave_service.rs:12`, `:58` |
| LV-09 | **Holidays.** The holiday calendar HR maintains. | *HR-managed at runtime* | `db` | `server/migrations/0013_leave.sql` |
| LV-10 | **Monthly leave cap.** An employee may take at most **2 leave days per calendar month**. ⚠️ New policy — not enforced yet (see §12-F). | **2 days / month** | `code` | *(to implement — §12-F)* |
| LV-11 | **Annual carry-over cap.** Up to **10 unused leave days** carry forward into the next annual year; the remainder lapses. ⚠️ New policy — LV-07 carries none today (see §12-G). | **≤ 10 days / year** | `code` | *(to implement — §12-G)* |
| LV-12 | **Maternity / Paternity eligibility.** Maternity and paternity leave apply **only after 1 year** of employment. ⚠️ New policy — not enforced yet (see §12-H). | **≥ 1 year tenure** | `code` | *(to implement — §12-H)* |

---

## 4. Working hours, meeting mode & grace time

| ID | Rule | Value (HR-editable) | Change type | System binding |
|---|---|---|---|---|
| HRS-01 | **Interval kinds.** How every tracked segment is classified. | `active · idle · meeting · break` | `code` | `server/src/db/intervals.rs:20` |
| HRS-02 | **"Worked" time** (weekly totals, reconcile). | `active + meeting` (idle & break excluded) | `code` | `server/src/db/intervals.rs:229` |
| HRS-03 | **Dashboard "day's work"** shown to users. ⚠️ Differs from HRS-02 — **includes idle** (only break excluded). See §12-E. | `active + idle + meeting` | `code` | `server/src/db/intervals.rs:75` |
| HRS-04 | **Idle threshold.** No OS input for this long ⇒ segment tagged `idle`. | **5m** (300 s) | `env` `TIMETRACKER_IDLE_THRESHOLD_SECS` | `apps/desktop/src-tauri/src/idle.rs:16` |
| HRS-05 | **Idle does NOT auto-pause.** The timer keeps running; idle time is tagged, not stopped. | No auto-pause | `code` | `apps/desktop/src-tauri/src/timer.rs:74` |
| HRS-06 | **Meeting mode.** Manually toggled; **counts as worked time**. Screenshots ARE captured during meetings (tagged `meeting`) so they appear in the gallery, but the AI never samples or analyses them. | Manual; counts as worked; meeting shots captured, labelled, not analysed | `code` | `apps/desktop/src-tauri/src/screenshot.rs:72`; sampler `server/src/sampler.rs:80`; analyzer `server/src/vision_analyzer.rs:51` |
| HRS-07 | **Weekly expected hours.** `working_days × this`; a full 5-day week = 40h. Approved leave/holidays reduce `working_days`. | **8h/working day** (28800 s) | `code` | `server/src/weekly_hours_service.rs:22` |
| HRS-08 | **Grace time (manual weekly hours).** HR/PM may add time to an employee's **current week**; a **reason is required**; positive only. | Enabled; reason required | `code` | `server/src/routes/time_grants.rs:46` |
| HRS-09 | **Grace time cap** per grant. | **1 week** (604800 s) | `code` | `server/src/routes/time_grants.rs:28` |

---

## 5. Desktop app behavior

| ID | Rule | Value (HR-editable) | Change type | System binding |
|---|---|---|---|---|
| DSK-01 | **Screenshot cadence.** Next capture at a random point in `[min, max]` after the last (≈5 min average); only while `working`. | min **150 s** / max **450 s** | `env` `TIMETRACKER_SCREENSHOT_MIN_INTERVAL_SECS` / `TIMETRACKER_SCREENSHOT_INTERVAL_SECS` | `apps/desktop/src-tauri/src/screenshot.rs:30` |
| DSK-02 | **Screenshot JPEG quality.** | **70** | `code` | `apps/desktop/src-tauri/src/screenshot.rs:72` |
| DSK-03 | **Break reminder cadence.** Repeats every interval until the break is paused/ended; a new break re-enables it. | **3m** (180 s) | `code` | `apps/desktop/src-tauri/src/presence.rs:20` |
| DSK-04 | **"You haven't started the timer" nudge.** Fires only when signed-in, machine active (not idle), timer stopped. | **5m** (300 s) | `code` | `apps/desktop/src-tauri/src/presence.rs:23` |
| DSK-05 | **Presence heartbeat interval.** | **45 s** | `code` | `apps/desktop/src-tauri/src/presence.rs:18` |
| DSK-06 | **Offline / stale presence.** A user shows `not_logged_in` if no heartbeat for this long. | **90 s** | `code` | `server/src/db/presence.rs:12` |

---

## 6. Tasks

| ID | Rule | Value (HR-editable) | Change type | System binding |
|---|---|---|---|---|
| TSK-01 | **Task importance weight** range + default. | **1–10**, default **5** | `code` | `server/migrations/0030_manual_tasks_weight_due_date.sql:4`; `server/src/db/manual_tasks.rs:24` |
| TSK-02 | **Due date.** Optional calendar day. (Known limitation: cannot be cleared back to open-ended once set.) | Optional | `code` | `server/src/db/manual_tasks.rs:150` |
| TSK-03 | **Task statuses.** | `open · done` | `code` | `server/src/db/manual_tasks.rs:11` |
| TSK-04 | **Who can assign.** Tasks are created by HR/PM and assigned to an employee. | HR & PM | `code` | `server/src/db/manual_tasks.rs:60` |

---

## 7. Employment types & roles (RBAC)

| ID | Rule | Value (HR-editable) | Change type | System binding |
|---|---|---|---|---|
| EMP-01 | **Employment types.** Set by HR; orthogonal to login role (contractors/interns still sign in as `employee`). | `employee · contractor · intern` (default `employee`) | `code` | `server/src/employment_type.rs:15`; `server/migrations/0031_employment_type.sql:7` |
| ROL-01 | **Roles.** The three login roles. ⚠️ CLAUDE.md calls these employee/manager/admin — the code uses these names; see §12-A. | `employee · project_manager · hr` | `code` | `server/src/role.rs:14` |
| ROL-02 | **Employee** — track time; view own hours/screenshots/reports/leave/tasks; desktop only. | Baseline | `code` | `server/src/middleware.rs:73` |
| ROL-03 | **Project Manager** — employee perms **plus** view/act on employees they manage (via `user_managers`): approve their team's leave, grant time, view team screenshots. **Cannot** configure leave/holidays or act org-wide. | Team-scoped | `code` | `server/src/middleware.rs:78`; scoping `routes/leave.rs:106` |
| ROL-04 | **HR** — full org-wide access; configure leave types/allocations/holidays; manage users; view audit logs. HR is the top privilege (there is no separate "admin"). | Full | `code` | `server/src/middleware.rs:88` |
| ROL-05 | **Screenshot visibility.** Employee: own only. PM: employees they manage. HR: all. | own / managed / all | `code` | `server/src/routes/uploads.rs:122`; `upload_service.rs:36` |

---

## 8. Authentication & security policy

| ID | Rule | Value (HR-editable) | Change type | System binding |
|---|---|---|---|---|
| AUTH-01 | **Access-token lifetime.** Default 1 h (hard cap 1 h). Longer = the desktop refreshes less often = fewer "session expired" prompts. | **3600 s** (cap 3600 s) | `env` `JWT_ACCESS_TTL_SECONDS` | `server/src/config.rs:79` |
| AUTH-02 | **Refresh-token lifetime.** Sliding window, rotated on every use — a user who opens the app within this window never re-logs-in. | **90d** (7776000 s) | `env` `JWT_REFRESH_TTL_SECONDS` | `server/src/config.rs:96` |
| AUTH-03 | **Refresh reuse grace.** A re-presented just-rotated token is recovered within this window; a genuine reuse outside it revokes all of that user's sessions. | **120 s** | `code` | `server/src/auth.rs:271` |
| AUTH-04 | **Password minimum length.** New password must also differ from the current one. No character-class rule. | **8** chars | `code` | `server/src/auth.rs:89` |
| AUTH-05 | **Password hashing.** | Argon2id, random salt, PHC-encoded | `code` | `server/src/auth.rs:45` |
| AUTH-06 | **Auto-generated temp password length** (new-employee credentials). | **20** chars (CSPRNG) | `code` | `server/src/auth.rs:33` |
| AUTH-07 | **JWT signing.** HS256 by default (secret ≥ 32 chars); RS256 + published JWKS available for the HRMS integration. | HS256 / RS256 | `env` `JWT_SIGNING_ALG` | `server/src/config.rs:100`; `server/src/jwt.rs` |
| SS-01 | **Presigned URL — upload (PUT).** | **15m** (900 s) | `code` | `server/src/upload_service.rs:12` |
| SS-02 | **Presigned URL — view (GET).** | **120 s** | `code` | `server/src/routes/uploads.rs:23` |
| SS-03 | **Presigned URL — AI fetch.** Plus a 15 MiB JPEG size cap + magic-byte check. | **300 s** | `code` | `server/src/storage.rs:219` |
| SS-04 | **Server never stores screenshot bytes** — metadata only; bytes live in GCS (CLAUDE.md Rule 5). | Enforced | `code` | (architecture) |

---

## 9. Audit & data retention

| ID | Rule | Value (HR-editable) | Change type | System binding |
|---|---|---|---|---|
| AUD-01 | **Audit log immutability.** All UPDATE/DELETE on `audit_logs` are blocked by a DB trigger — except nulling `actor_id` when a user is deleted. | Immutable | `code` | `server/migrations/0036_audit_allow_actor_delete.sql:12` |
| AUD-02 | **Audit-log retention.** ⚠️ **None implemented** — logs are kept forever. Set a target here if HR wants a retention window. | **— (keep forever)** *(target: HR decision)* | `code` | *(no binding yet — see §12-B)* |
| AUD-03 | **Screenshot / data retention.** ⚠️ **None implemented.** No auto-deletion of screenshots or intervals exists. Set a target here if HR wants one. | **— (no auto-delete)** *(target: HR decision)* | `code` | *(no binding yet — see §12-B)* |

---

## 10. Where to change what (quick map for HR)

- **Change a number now, no code (env):** ATT-01, HRS-04, DSK-01, AUTH-01, AUTH-02, AUTH-07 — edit the VM `.env` and restart the server/desktop build.
- **Change through the portal (db):** LV-01…LV-05, LV-09 — use the HR leave-settings screens.
- **Everything else (code):** needs a one-line PR + the next deploy. The agent can open that PR from a Value change here.

---

## 11. Change log

| Date | Editor | Rules changed | Note |
|---|---|---|---|
| 2026-07-27 | Shlok | — | Initial OKF captured from the running system (code-verified). |
| 2026-07-27 | HR | LV-10, LV-11, LV-12 | Added monthly cap (2/mo), carry-over cap (10/yr), maternity/paternity 1-yr eligibility. New policy — pending build (§12-F/G/H). |
| 2026-07-29 | Shlok | AUTH-01, AUTH-02 | Access token 5 min → 1 h; refresh token 30 d → 90 d, to cut desktop "session expired" prompts. Ships on next server deploy. |
| 2026-07-29 | Shlok | ATT-07 | Weekends never count as present/partial even with tracked time; auto-present skipped on weekends. Ships on next server deploy. |
| 2026-07-30 | Shlok | HRS-06 | Meeting mode now captures screenshots (labelled "meeting"), but the AI still never analyses them. Ships in the next desktop release. |
| 2026-07-30 | Shlok | ATT-07 | Backfill (migration 0039): existing weekend days saved as present/partial are corrected to "weekend" (HR overrides untouched), so the rule applies to past data too. |
| 2026-08-03 | Shlok | HRS-07 | Weekly shortfall mail consolidated: ONE company-wide digest to HR listing every employee below their required hours, plus one team digest per PM — instead of a separate mail per employee. Threshold unchanged (8h × working days, Mon–Fri, minus holidays/leave). |

*(HR: add a row whenever you edit a Value. The agent appends a row for every reconciliation it performs.)*

---

## 12. Known OKF ↔ system drifts (open reconciliation items)

These are real mismatches found while capturing the OKF. Each is something the reconciliation
agent (or a human) should resolve; until then the OKF records the **intended** value and the
**actual** behavior.

- **A. Role names.** `CLAUDE.md` (§Roles) says *employee / manager / admin*; the code uses
  *employee / project_manager / hr* and has **no separate admin tier** (HR is top). PM scoping is
  via `user_managers`, not `team_id`. → Fix the doc, not the code (code is correct).
- **B. Retention rules.** `CLAUDE.md` lists "configure retention rules" as an admin capability,
  but **no retention/purge logic exists** for audit logs (AUD-02), screenshots, or intervals
  (AUD-03). → Either build it or drop the claim. OKF currently records "keep forever".
- **C. Attendance day boundary.** Attendance rolls up on **UTC midnight** (ATT-05) while the
  dashboard/hours/grace use **4 AM Asia/Kolkata** (`intervals.rs:104`). Two different "days".
  → Decide one boundary and make both use it.
- **D. Grace time vs compliance.** Grace hours (HRS-08) show in the dashboard week total but are
  **not** counted in the weekly-hours shortfall check (`weekly_hours_service.rs` uses attendance
  worked-seconds only). → Decide whether grace should satisfy the weekly target.
- **E. Two "hours" definitions.** Dashboard day (HRS-03) includes idle; "worked" (HRS-02) excludes
  it. Both are intentional today but easy to confuse in reports. → Confirm the intended one per surface.
- **F. Monthly leave cap (LV-10).** New HR policy: at most 2 leave days per calendar month. Not yet
  enforced — needs a per-calendar-month tally + rejection at request time in `leave_service.rs`
  (alongside the existing balance check). → Build.
- **G. Annual carry-over (LV-11).** New HR policy: carry up to 10 unused days into the next year;
  the rest lapses. LV-07 currently carries none. Needs a year-rollover step that seeds the new
  year's allocation with `min(unused, 10)`. → Build.
- **H. Maternity / Paternity tenure (LV-12).** New HR policy: eligible only after 1 year of
  employment. Needs a tenure check (join date vs. request start) gating those leave types — and a
  reliable **employment start date** on `users` if one isn't already stored. → Build.

---

*End of OKF v1.0 — TimeTracker / RUH. This file is the source of truth for policy values;
the code is the source of truth for behavior. The reconciliation agent's job is to keep them equal.*
