# -*- coding: utf-8 -*-
"""Generate the TimeTracker feature implementation plan as a PDF."""
from reportlab.lib.pagesizes import A4
from reportlab.lib.units import mm
from reportlab.lib import colors
from reportlab.lib.styles import getSampleStyleSheet, ParagraphStyle
from reportlab.lib.enums import TA_LEFT
from reportlab.platypus import (
    SimpleDocTemplate, Paragraph, Spacer, Table, TableStyle, ListFlowable,
    ListItem, HRFlowable, PageBreak, KeepTogether,
)

NAVY = colors.HexColor("#1f3a5f")
BLUE = colors.HexColor("#2f6fb0")
LIGHT = colors.HexColor("#eef3f8")
GREY = colors.HexColor("#666666")
GREEN = colors.HexColor("#2e7d32")

ss = getSampleStyleSheet()
styles = {}
styles["title"] = ParagraphStyle("title", parent=ss["Title"], fontSize=24,
                                  textColor=NAVY, spaceAfter=4, leading=28)
styles["subtitle"] = ParagraphStyle("subtitle", parent=ss["Normal"], fontSize=11,
                                     textColor=GREY, spaceAfter=2, leading=15)
styles["h1"] = ParagraphStyle("h1", parent=ss["Heading1"], fontSize=16, textColor=colors.white,
                               backColor=NAVY, borderPadding=(6, 8, 6, 8), spaceBefore=18,
                               spaceAfter=10, leading=20)
styles["h2"] = ParagraphStyle("h2", parent=ss["Heading2"], fontSize=12.5, textColor=NAVY,
                               spaceBefore=10, spaceAfter=4, leading=16)
styles["h3"] = ParagraphStyle("h3", parent=ss["Heading3"], fontSize=11, textColor=BLUE,
                               spaceBefore=8, spaceAfter=3, leading=14)
styles["body"] = ParagraphStyle("body", parent=ss["Normal"], fontSize=9.8, leading=14,
                                 spaceAfter=5, alignment=TA_LEFT)
styles["small"] = ParagraphStyle("small", parent=ss["Normal"], fontSize=8.5, leading=12,
                                  textColor=GREY)
styles["goal"] = ParagraphStyle("goal", parent=ss["Normal"], fontSize=9.8, leading=14,
                                 backColor=LIGHT, borderPadding=(6, 8, 6, 8), spaceAfter=6)
styles["code"] = ParagraphStyle("code", parent=ss["Code"], fontSize=8.3, leading=11,
                                 textColor=colors.HexColor("#11324d"))
styles["bullet"] = ParagraphStyle("bullet", parent=ss["Normal"], fontSize=9.6, leading=13)
styles["step"] = ParagraphStyle("step", parent=ss["Normal"], fontSize=9.6, leading=13.5)

story = []


def P(text, style="body"):
    story.append(Paragraph(text, styles[style]))


def H1(text):
    story.append(Paragraph(text, styles["h1"]))


def H2(text):
    story.append(Paragraph(text, styles["h2"]))


def H3(text):
    story.append(Paragraph(text, styles["h3"]))


def goal(text):
    story.append(Paragraph("<b>Goal.</b> " + text, styles["goal"]))


def gap(h=6):
    story.append(Spacer(1, h))


def rule():
    story.append(HRFlowable(width="100%", thickness=0.6, color=colors.HexColor("#cdd8e4"),
                            spaceBefore=6, spaceAfter=6))


def steps(items, start=1):
    flow = [ListItem(Paragraph(t, styles["step"]), value=start + i) for i, t in enumerate(items)]
    story.append(ListFlowable(flow, bulletType="1", leftIndent=16, bulletColor=NAVY))
    gap(4)


def bullets(items):
    flow = [ListItem(Paragraph(t, styles["bullet"])) for t in items]
    story.append(ListFlowable(flow, bulletType="bullet", leftIndent=14,
                              bulletColor=BLUE, start="square"))
    gap(4)


def table(headers, rows, col_widths):
    data = [[Paragraph("<b>%s</b>" % h, styles["small"]) for h in headers]]
    for r in rows:
        data.append([Paragraph(c, styles["small"]) for c in r])
    t = Table(data, colWidths=col_widths, hAlign="LEFT")
    t.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, 0), NAVY),
        ("TEXTCOLOR", (0, 0), (-1, 0), colors.white),
        ("FONTSIZE", (0, 0), (-1, -1), 8.3),
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("GRID", (0, 0), (-1, -1), 0.4, colors.HexColor("#c3d0de")),
        ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, LIGHT]),
        ("LEFTPADDING", (0, 0), (-1, -1), 5),
        ("RIGHTPADDING", (0, 0), (-1, -1), 5),
        ("TOPPADDING", (0, 0), (-1, -1), 3),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 3),
    ]))
    # recolor header paragraphs white
    for i, h in enumerate(headers):
        data[0][i] = Paragraph('<font color="white"><b>%s</b></font>' % h, styles["small"])
    story.append(t)
    gap(6)


# ----------------------------------------------------------------------------
# Cover
# ----------------------------------------------------------------------------
P("TimeTracker — Feature Implementation Plan", "title")
P("Step-wise specification for AI work-verification reporting, HR (Horilla-style) "
  "modules, multi-team tracking, HR-assigned tasks, and day-to-day screenshot review.", "subtitle")
P("Prepared: 11 June 2026 &nbsp;|&nbsp; Stack: Rust / Axum / PostgreSQL / SQLx &middot; "
  "Next.js / TypeScript / Tailwind / shadcn &middot; Tauri 2 desktop &middot; Cloudflare R2 / MinIO &middot; "
  "Gemini 2.5 Flash", "subtitle")
rule()

H2("0. Context and conventions")
P("This plan extends the existing platform. Already implemented (STEP 0&ndash;10): role-based auth "
  "(employee / project_manager / hr), immutable UTC time intervals with a local-first SQLite sync "
  "queue, presence and idle detection with statuses <b>working / idle / break / meeting / not_working / "
  "not_logged_in</b>, screenshot capture with metadata-only storage and short-lived presigned URLs, the "
  "daily screenshot sampler (analysis_jobs / analysis_job_samples), and the Vision AI analyzer "
  "(analysis_results) comparing screenshots to assigned Linear tickets with Gemini 2.5 Flash.")
P("Every feature below follows the project's standard workflow (CLAUDE.md): "
  "<b>1)</b> design schema &rarr; <b>2)</b> migration &rarr; <b>3)</b> repository &rarr; "
  "<b>4)</b> service &rarr; <b>5)</b> API route &rarr; <b>6)</b> frontend &rarr; "
  "<b>7)</b> tests &rarr; <b>8)</b> docs. Cross-cutting rules apply throughout: RBAC enforced "
  "server-side, all timestamps stored in UTC, SQLx compile-time-checked queries with migrations, "
  "audit logging of sensitive actions, presigned short-lived URLs (storage keys never exposed), and "
  "no secrets sent to clients.")

# ----------------------------------------------------------------------------
# Feature 1
# ----------------------------------------------------------------------------
H1("Feature 1 &mdash; AI screenshot analysis report on the HR dashboard")
goal("Aggregate the per-screenshot AI verdicts (already produced in STEP 10) into a per-employee, "
     "per-day <b>report</b> that scores how well the day's activity matched the employee's assigned "
     "ticket(s)/task(s), and let HR review it on the dashboard.")
H2("Schema and migration")
table(["Table", "Key columns", "Notes"],
      [["analysis_reports",
        "id, user_id&rarr;users, day (DATE), job_id&rarr;analysis_jobs, total_analyzed, "
        "aligned_count, partially_count, not_aligned_count, inconclusive_count, "
        "alignment_score (0&ndash;100), summary_text, model, created_at",
        "One row per (user, day). UNIQUE(user_id, day). Built from analysis_results."]],
      [70 * mm, 75 * mm, 33 * mm])
H2("Steps")
steps([
    "Add migration <font face='Courier'>00xx_analysis_reports.sql</font> with the table above "
    "(UNIQUE(user_id, day), FK cascade on user/job).",
    "Repository <font face='Courier'>db/analysis_reports.rs</font>: <font face='Courier'>upsert(report)</font>, "
    "<font face='Courier'>get(user_id, day)</font>, <font face='Courier'>list(scope, day_range)</font>.",
    "Service <font face='Courier'>report_service.rs</font>: aggregate <font face='Courier'>analysis_results</font> "
    "for the day's job &rarr; counts + <b>alignment_score</b> = weighted(aligned=1, partial=0.5, "
    "not_aligned=0, inconclusive excluded) &times; 100; then call Gemini (text) to compose a short "
    "natural-language <b>summary_text</b> from the per-shot rationales.",
    "Trigger: run report generation at the end of <font face='Courier'>analyze_day</font>, and via the "
    "scheduled daily job (Feature sequencing &sect;7), so reports refresh idempotently.",
    "API: <font face='Courier'>GET /admin/users/:id/report?day=</font> (HR / PM team-scoped), "
    "<font face='Courier'>GET /admin/reports?day=&amp;team=</font> (roster of reports), "
    "<font face='Courier'>GET /me/report?day=</font> (employee's own).",
    "Frontend (HR dashboard): a <b>Reports</b> view &mdash; per-employee report card showing the alignment "
    "score, a verdict-breakdown bar/donut, the day's sampled screenshots with their individual verdicts, "
    "and the AI summary. Drill-down from the team table.",
    "Tests: aggregation math (counts &amp; score), threshold/exclusion of inconclusive, RBAC gating; "
    "acceptance: given a day with analysed screenshots, an HR user sees a stored, scored report.",
])

# ----------------------------------------------------------------------------
# Feature 2
# ----------------------------------------------------------------------------
H1("Feature 2 &mdash; Exclude meeting-mode screenshots from analysis")
goal("Screenshots captured while the employee is in <b>meeting</b> mode must be stored and viewable but "
     "never sampled or analysed by the AI.")
H2("Steps")
steps([
    "Tag capture context: add a <font face='Courier'>captured_status</font> column to "
    "<font face='Courier'>screenshots</font> (working / meeting / break / &hellip;) via migration; "
    "default 'working' for existing rows.",
    "Desktop: include the current presence status/kind when posting screenshot metadata "
    "(<font face='Courier'>screenshot.rs</font> / upload notify).",
    "Sampler (STEP 9): tighten eligibility so only <font face='Courier'>captured_status = 'working'</font> "
    "screenshots enter the bucket selection &mdash; meeting/break shots are skipped.",
    "Analyzer (STEP 10): defensive guard &mdash; never send a non-working screenshot to Gemini even if "
    "passed in.",
    "Tests: a day with mixed working/meeting shots samples only working ones; meeting shots produce no "
    "<font face='Courier'>analysis_results</font> row. Acceptance: meeting screenshots are excluded from "
    "all analysis while remaining visible in the gallery (Feature 3).",
])

# ----------------------------------------------------------------------------
# Feature 3
# ----------------------------------------------------------------------------
H1("Feature 3 &mdash; Day-to-day screenshot &amp; summary view with filtering")
goal("Present screenshots and their analysis summary in a per-day format; applying a filter "
     "(date / employee / team / verdict) shows the matching summary.")
H2("Steps")
steps([
    "Backend: day-grouped listings &mdash; <font face='Courier'>GET /admin/users/:id/screenshots?day=</font> "
    "and <font face='Courier'>GET /me/screenshots?day=</font>, each returning presigned view URLs plus the "
    "per-shot verdict (joined from analysis_results) and a <b>meeting</b> flag.",
    "Filtering params on the report/listing endpoints: <font face='Courier'>day</font> or "
    "<font face='Courier'>from</font>/<font face='Courier'>to</font>, <font face='Courier'>team</font>, "
    "<font face='Courier'>verdict</font>, <font face='Courier'>user</font>; the daily summary recomputes "
    "for the active filter.",
    "Frontend: a day navigator (calendar/date-picker) on both the employee and HR dashboards. Selecting a "
    "day shows that day's screenshots (meeting ones badged &ldquo;not analysed&rdquo;) plus that day's "
    "summary/report. Filter controls refresh the summary in place.",
    "Tests: listings are correctly bucketed by day in UTC&rarr;local boundary; filters return only matching "
    "rows; meeting shots shown but marked. Acceptance: choosing a date or filter yields the relevant "
    "screenshots and the matching summary.",
])

# ----------------------------------------------------------------------------
# Feature 4
# ----------------------------------------------------------------------------
H1("Feature 4 &mdash; Multi-team tracking, team selection before timer, team summary")
goal("An employee may belong to multiple teams; before starting the timer they pick the team they are "
     "working for from a dropdown; HR/admin assign teams; the admin dashboard gains a <b>Team Summary</b> "
     "showing the whole team's activity and each member's individual summary.")
H2("Schema and migration")
table(["Table", "Key columns", "Notes"],
      [["teams", "id, name, description, created_at", "Team catalogue."],
       ["user_teams", "user_id&rarr;users, team_id&rarr;teams, PRIMARY KEY(user_id, team_id)",
        "Many-to-many membership (one employee &rarr; many teams)."],
       ["intervals (alter)", "+ team_id&rarr;teams (nullable)",
        "Tags each tracked interval with the team it was logged under."]],
      [40 * mm, 95 * mm, 43 * mm])
H2("Steps")
steps([
    "Migration: create <font face='Courier'>teams</font> and <font face='Courier'>user_teams</font>; "
    "add nullable <font face='Courier'>team_id</font> to <font face='Courier'>intervals</font>. "
    "(Supersedes the single <font face='Courier'>users.team_id</font> for membership.)",
    "Repositories: <font face='Courier'>db/teams.rs</font> (CRUD, list_members), membership add/remove; "
    "extend interval insert to carry <font face='Courier'>team_id</font>.",
    "Admin/HR API + UI: create/edit teams, assign/unassign employees (an employee can be added to several "
    "teams). Extend the Manage Users page.",
    "Employee dashboard: a <b>required team dropdown</b> (populated from the employee's memberships) shown "
    "before <b>Start</b>; the chosen <font face='Courier'>team_id</font> is passed to the start-tracking "
    "command and stamped on every interval of that session. If the employee has exactly one team, "
    "preselect it.",
    "Admin <b>Team Summary</b> section: <font face='Courier'>GET /admin/teams</font> and "
    "<font face='Courier'>GET /admin/teams/:id/summary</font> returning team-level aggregates (total worked, "
    "status breakdown, who is active now) plus each member's individual totals; UI shows the team roll-up "
    "and expandable per-member summaries.",
    "Tests: multi-team membership, interval team tagging, team aggregate math, PM scope. Acceptance: an "
    "employee in 2+ teams selects one before starting; the admin sees both the combined team summary and "
    "each individual.",
])

# ----------------------------------------------------------------------------
# Feature 5
# ----------------------------------------------------------------------------
H1("Feature 5 &mdash; HR-assigned manual tasks analysed as tickets by the AI")
goal("HR can add a task directly to any employee's dashboard. It is not a real Linear ticket, but it is "
     "fed to the Vision AI as task context so screenshots are analysed against it just like a ticket.")
H2("Schema and migration")
table(["Table", "Key columns", "Notes"],
      [["manual_tasks",
        "id, user_id&rarr;users (assignee), created_by&rarr;users (HR), title, description, "
        "status (open/done), created_at, updated_at",
        "HR-authored pseudo-tickets shown on the employee dashboard."]],
      [42 * mm, 96 * mm, 40 * mm])
H2("Steps")
steps([
    "Migration: <font face='Courier'>manual_tasks</font> as above.",
    "API: HR-only create/list/update/delete for a target employee (audit-logged); "
    "<font face='Courier'>GET /me/tasks</font> for the employee's own assigned tasks.",
    "AI integration: the analyzer's context builder merges <font face='Courier'>manual_tasks</font> with "
    "Linear tickets, mapping each task to the same context shape "
    "(<font face='Courier'>id, title, state, labels=[], description_excerpt</font>) with an id prefix such "
    "as <font face='Courier'>task:&lt;uuid&gt;</font>; <font face='Courier'>matched_ticket</font> may then "
    "reference a manual task.",
    "Frontend: employee dashboard lists assigned manual tasks alongside the tickets panel; HR UI to add/"
    "close tasks per employee.",
    "Tests: task appears in context, can be matched, RBAC (only HR creates). Acceptance: an HR-added task "
    "is analysed and can be the matched item in a screenshot verdict / report.",
])

# ----------------------------------------------------------------------------
# Feature 6
# ----------------------------------------------------------------------------
H1("Feature 6 &mdash; Horilla-style HR portal: Onboarding, Leave, Attendance, Payroll")
P("These mirror the corresponding modules of the open-source <b>Horilla HRMS</b> "
  "(github.com/horilla/horilla-hr). Horilla is a Django/MySQL application; we reimplement the same "
  "feature set and workflows natively in our Rust + Next.js stack (we do not import Django code). Each "
  "sub-module follows the standard 8-step workflow and lives in the HR portal with HR-only configuration "
  "and employee self-service where applicable.", "body")

H2("6A. Onboarding")
goal("Recruit-to-hire pipeline: candidates move through onboarding stages with a task checklist and "
     "document collection, then convert into employee accounts.")
H3("Schema")
bullets([
    "<font face='Courier'>candidates</font> (id, name, email, role/position, source, stage_id, status, hired_at)",
    "<font face='Courier'>onboarding_stages</font> (id, name, sequence) &mdash; e.g. Applied, Interview, Offer, Onboarding, Hired",
    "<font face='Courier'>onboarding_tasks</font> (id, stage_id, title) and "
    "<font face='Courier'>candidate_tasks</font> (candidate_id, task_id, done, done_at)",
    "<font face='Courier'>candidate_documents</font> (id, candidate_id, doc_type, storage_key)",
])
H3("Steps")
steps([
    "Migrations for the tables above; seed default stages and a default task checklist.",
    "Repositories + service: stage transitions, checklist completion, document upload (presigned, Rule 5).",
    "API (HR): candidate CRUD, move stage, toggle task, upload docs; <b>convert candidate &rarr; user</b> "
    "(creates the employee account, optionally triggers a reset-password / invite).",
    "Frontend: a Kanban board of stages with candidate cards and per-candidate checklist + documents.",
    "Tests + acceptance: a candidate progresses through all stages and is converted into an employee.",
])

H2("6B. Leave management")
goal("Leave types with per-employee balances, leave requests with an approval workflow, and a company "
     "holiday calendar.")
H3("Schema")
bullets([
    "<font face='Courier'>leave_types</font> (id, name, paid, default_days, carry_forward)",
    "<font face='Courier'>leave_allocations</font> (id, user_id, leave_type_id, year, allotted_days, used_days)",
    "<font face='Courier'>leave_requests</font> (id, user_id, leave_type_id, start_date, end_date, days, "
    "reason, status [pending/approved/rejected], approver_id, decided_at)",
    "<font face='Courier'>holidays</font> (id, date, name) and optional company-wide leave days",
])
H3("Steps")
steps([
    "Migrations + seed common leave types (casual, sick, earned) and the year's holidays.",
    "Service: balance computation (allotted &minus; used), overlap/holiday-aware day counting, approval "
    "workflow with audit logging.",
    "API: employee request/cancel leave + view balance; manager/HR approve/reject; HR configure types, "
    "allocations, holidays.",
    "Frontend: employee leave form + balance widget; HR/manager approvals queue; holiday calendar.",
    "Tests + acceptance: an employee submits a request, balance reserves the days, an approver decides, and "
    "the balance/attendance reflect the outcome.",
])

H2("6C. Attendance")
goal("Daily attendance derived primarily from the existing interval engine (check-in = first start, "
     "check-out = last stop), with present/absent/half-day/leave status and reports.")
H3("Schema")
bullets([
    "<font face='Courier'>attendance_days</font> (id, user_id, day, first_in, last_out, worked_seconds, "
    "status [present/absent/half_day/leave/holiday]) &mdash; cached roll-up of intervals",
    "optional <font face='Courier'>shifts</font> (id, name, start_time, end_time, grace_minutes) for "
    "late-in / early-out flags",
])
H3("Steps")
steps([
    "Service: build <font face='Courier'>attendance_days</font> from intervals per day; mark days covered "
    "by approved leave (Feature 6B) or holidays; compute late/early against the shift if configured.",
    "Trigger: nightly roll-up job + on-demand recompute for a date range.",
    "API: employee own attendance; HR/manager team attendance report (range, export).",
    "Frontend: attendance calendar/table per employee and a team report; integrates leave + holidays.",
    "Tests + acceptance: a worked day yields a present record with correct in/out and worked hours; an "
    "approved-leave day shows as leave, not absent.",
])

H2("6D. Payroll")
goal("Per-employee salary structure, periodic payroll runs that generate payslips (incorporating "
     "attendance / loss-of-pay and leave), and employee payslip access.")
H3("Schema")
bullets([
    "<font face='Courier'>salary_structures</font> (id, user_id, basic, allowances_json, deductions_json, "
    "effective_from)",
    "<font face='Courier'>payroll_runs</font> (id, period_start, period_end, status, run_at, run_by)",
    "<font face='Courier'>payslips</font> (id, payroll_run_id, user_id, gross, deductions, net, "
    "lop_days, breakdown_json, generated_at)",
])
H3("Steps")
steps([
    "Migrations for salary structures, payroll runs, payslips.",
    "Service: for a period, pull attendance + approved leave to compute loss-of-pay, apply the salary "
    "structure (basic + allowances &minus; deductions &minus; LOP) &rarr; gross / deductions / net.",
    "API (HR): configure salary; run payroll for a period; (re)generate payslips; employee "
    "<font face='Courier'>GET /me/payslips</font>. Payslip PDF export reuses this PDF tooling.",
    "Frontend: HR payroll run screen + payslip list; employee payslip download.",
    "Tests + acceptance: a payroll run for a period produces a payslip per active employee with correct net "
    "pay reflecting attendance and leave.",
])

# ----------------------------------------------------------------------------
# Sequencing
# ----------------------------------------------------------------------------
H1("7. Suggested build sequence and dependencies")
P("Ordered to unblock dependencies and ship value incrementally:", "body")
table(["#", "Feature", "Why this order / depends on"],
      [["1", "F2 &mdash; meeting-mode exclusion + screenshot status tagging",
        "Small; makes all downstream analysis correct. No dependencies."],
       ["2", "F1 + F3 &mdash; daily report aggregation, HR report view, day-to-day filtering",
        "Builds on STEP 10 + F2. Delivers the core HR-visible reporting."],
       ["3", "F4 &mdash; multi-team, team selection, team summary",
        "Independent; enriches reports/attendance with team attribution."],
       ["4", "F5 &mdash; HR manual tasks &rarr; AI",
        "Plugs into the analyzer's context builder (post-F1)."],
       ["5", "F6 &mdash; Horilla modules: Onboarding &rarr; Leave &rarr; Attendance &rarr; Payroll",
        "Largest. Attendance reuses intervals (+F4 teams); Payroll depends on Attendance + Leave."]],
      [8 * mm, 78 * mm, 92 * mm])

H1("8. Cross-cutting requirements (apply to every feature)")
bullets([
    "<b>RBAC</b>: employee / project_manager / HR (admin) enforced server-side; employees see only their "
    "own data, managers their team, HR everything.",
    "<b>Data</b>: UTC storage (convert in UI only); SQLx compile-time-checked queries; a migration per "
    "schema change; repository pattern; no <font face='Courier'>unwrap()</font> in production code.",
    "<b>Security</b>: audit-log sensitive actions (create/update/delete, approvals, payroll, screenshot "
    "access); short-lived presigned URLs only; secrets server-side and git-ignored.",
    "<b>Testing</b>: unit + integration tests per feature; &gt;80% coverage on critical modules; live "
    "end-to-end verification before sign-off.",
    "<b>Deliverables per feature</b> (CLAUDE.md): file-tree changes, migration files, tests, and a short "
    "architecture note.",
])

gap(8)
P("End of plan. This document is a specification of intended work; implementation of each feature will "
  "proceed step-by-step with verification, in the sequence of &sect;7.", "small")


def footer(canvas, doc):
    canvas.saveState()
    canvas.setFont("Helvetica", 8)
    canvas.setFillColor(GREY)
    canvas.drawString(20 * mm, 12 * mm, "TimeTracker — Feature Implementation Plan")
    canvas.drawRightString(190 * mm, 12 * mm, "Page %d" % doc.page)
    canvas.setStrokeColor(colors.HexColor("#cdd8e4"))
    canvas.line(20 * mm, 15 * mm, 190 * mm, 15 * mm)
    canvas.restoreState()


doc = SimpleDocTemplate(
    "TimeTracker_Feature_Plan.pdf", pagesize=A4,
    leftMargin=20 * mm, rightMargin=20 * mm, topMargin=18 * mm, bottomMargin=20 * mm,
    title="TimeTracker — Feature Implementation Plan", author="TimeTracker Team",
)
doc.build(story, onFirstPage=footer, onLaterPages=footer)
print("OK wrote TimeTracker_Feature_Plan.pdf")
