import { z } from "zod";
import { roleSchema, type Role } from "@timetracker/shared";
import { useAuthStore } from "@/lib/auth-store";

const API_BASE = process.env.NEXT_PUBLIC_API_BASE_URL ?? "http://localhost:9000";


const loginResponseSchema = z.object({
  access_token: z.string(),
  refresh_token: z.string(),
  token_type: z.string(),
  expires_in: z.number().optional(),
  user: z.object({
    id: z.string(),
    name: z.string(),
    email: z.string(),
    role: roleSchema,
    team: z.string().nullable(),
  }),
});

export type LoginResponse = z.infer<typeof loginResponseSchema>;

export const presenceStatusSchema = z.enum([
  "working",
  "idle",
  "break",
  "meeting",
  "not_working",
  "not_logged_in",
]);
export type PresenceStatus = z.infer<typeof presenceStatusSchema>;

const teamMemberSchema = z.object({
  user: z.object({
    id: z.string(),
    name: z.string(),
    email: z.string(),
    role: roleSchema,
  }),
  status: presenceStatusSchema,
  last_seen_at: z.string().nullable(),
  today_seconds: z.number(),
});
export type TeamMember = z.infer<typeof teamMemberSchema>;
const teamSchema = z.array(teamMemberSchema);

const hoursSummarySchema = z.object({
  total_seconds: z.number(),
  today_seconds: z.number(),
  week_seconds: z.number(),
  active_seconds: z.number(),
  idle_seconds: z.number(),
});
export type HoursSummary = z.infer<typeof hoursSummarySchema>;

const adminShotSchema = z.object({
  id: z.string(),
  taken_at: z.string(),
  url: z.string(),
});
export type AdminShot = z.infer<typeof adminShotSchema>;

/** Try to rotate the refresh token. Returns true if a new access token is set. */
async function tryRefresh(): Promise<boolean> {
  const rt = useAuthStore.getState().refreshToken;
  if (!rt) return false;
  try {
    const res = await fetch(`${API_BASE}/auth/refresh`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ refresh_token: rt }),
    });
    if (!res.ok) {
      useAuthStore.getState().clear();
      return false;
    }
    const data = (await res.json()) as { access_token: string; refresh_token: string };
    useAuthStore.getState().setTokens(data.access_token, data.refresh_token);
    return true;
  } catch {
    return false;
  }
}

/** Authenticated request that transparently refreshes the access token on 401
 *  and surfaces the server's `{ error }` message on failure. */
async function authedJson(method: string, path: string, body?: unknown): Promise<unknown> {
  const opts = (): RequestInit => {
    const headers: Record<string, string> = {
      Authorization: `Bearer ${useAuthStore.getState().token ?? ""}`,
    };
    const o: RequestInit = { method, headers };
    if (body !== undefined) {
      headers["content-type"] = "application/json";
      o.body = JSON.stringify(body);
    }
    return o;
  };

  let res = await fetch(`${API_BASE}${path}`, opts());
  if (res.status === 401 && (await tryRefresh())) {
    res = await fetch(`${API_BASE}${path}`, opts());
  }
  if (res.status === 401 || res.status === 403) {
    throw new Error("Not authorized.");
  }
  if (!res.ok) {
    let msg = `Request failed (status ${res.status}).`;
    try {
      const j = (await res.json()) as { error?: string };
      if (j.error) msg = j.error;
    } catch {
      /* non-JSON body */
    }
    throw new Error(msg);
  }
  const text = await res.text();
  return text ? JSON.parse(text) : null;
}

const authedGetJson = (path: string) => authedJson("GET", path);

/** Live team roster (`GET /admin/team`). */
export async function fetchTeam(): Promise<TeamMember[]> {
  return teamSchema.parse(await authedGetJson("/admin/team"));
}

/** Drill-down hours for one employee (`GET /admin/users/:id/hours`). */
export async function fetchUserHours(userId: string): Promise<HoursSummary> {
  return hoursSummarySchema.parse(await authedGetJson(`/admin/users/${userId}/hours`));
}

/** Drill-down screenshots for one employee (`GET /admin/users/:id/screenshots`). */
export async function fetchUserScreenshots(userId: string): Promise<AdminShot[]> {
  return z.array(adminShotSchema).parse(await authedGetJson(`/admin/users/${userId}/screenshots`));
}

// ---- Daily AI report (Feature 1) ----

const dailyReportSchema = z.object({
  user_id: z.string(),
  day: z.string(),
  total_analyzed: z.number(),
  aligned_count: z.number(),
  partially_count: z.number(),
  not_aligned_count: z.number(),
  inconclusive_count: z.number(),
  alignment_score: z.number(),
  summary_text: z.string(),
  model: z.string(),
  created_at: z.string(),
});
export type DailyReport = z.infer<typeof dailyReportSchema>;

/** A day's report for one employee (`GET /admin/users/:id/report?day=`). */
export async function fetchUserReport(userId: string, day: string): Promise<DailyReport | null> {
  const res = (await authedGetJson(`/admin/users/${userId}/report?day=${day}`)) as {
    report: unknown;
  };
  return res.report ? dailyReportSchema.parse(res.report) : null;
}

/** Run the AI analyzer on demand for one employee's day
 *  (`POST /admin/users/:id/analyze?day=`). Returns the counts. */
export async function analyzeUserDay(
  userId: string,
  day: string,
): Promise<{ analyzed: number; skipped: number }> {
  const res = (await authedJson("POST", `/admin/users/${userId}/analyze?day=${day}`)) as {
    analyzed: number;
    skipped: number;
  };
  return res;
}

// ---- Day-based screenshots with verdicts (Feature 3) ----

const dayShotSchema = z.object({
  screenshot: z.object({
    id: z.string(),
    taken_at: z.string(),
    captured_status: z.string(),
  }),
  verdict: z.string().nullable(),
  meeting_flag: z.boolean(),
  presigned_url: z.string(),
});
export type DayShot = z.infer<typeof dayShotSchema>;

/** A day's screenshots for one employee (`GET /admin/users/:id/screenshots?day=`). */
export async function fetchUserDayScreenshots(userId: string, day: string): Promise<DayShot[]> {
  return z
    .array(dayShotSchema)
    .parse(await authedGetJson(`/admin/users/${userId}/screenshots?day=${day}`));
}

// ---- Teams + summary (Feature 4) ----

const teamWithCountSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string(),
  created_at: z.string(),
  member_count: z.number(),
});
export type TeamWithCount = z.infer<typeof teamWithCountSchema>;

/** All teams with member counts (`GET /admin/teams`). */
export async function fetchTeams(): Promise<TeamWithCount[]> {
  return z.array(teamWithCountSchema).parse(await authedGetJson("/admin/teams"));
}

const teamSummarySchema = z.object({
  team: z.object({ id: z.string(), name: z.string(), description: z.string() }),
  total_seconds: z.number(),
  member_count: z.number(),
  active_users: z.number(),
  status_breakdown: z.object({
    active: z.number(),
    idle: z.number(),
    meeting: z.number(),
    break: z.number(),
  }),
  members: z.array(
    z.object({
      user_id: z.string(),
      name: z.string(),
      email: z.string(),
      worked_seconds: z.number(),
    }),
  ),
});
export type TeamSummary = z.infer<typeof teamSummarySchema>;

/** Team rollup (`GET /admin/teams/:id/summary`). */
export async function fetchTeamSummary(teamId: string): Promise<TeamSummary> {
  return teamSummarySchema.parse(await authedGetJson(`/admin/teams/${teamId}/summary`));
}

const userTeamSchema = z.object({
  id: z.string(),
  name: z.string(),
  description: z.string(),
  created_at: z.string(),
});
export type UserTeam = z.infer<typeof userTeamSchema>;

/** The teams an employee belongs to (`GET /admin/users/:id/teams`). */
export async function fetchUserTeams(userId: string): Promise<UserTeam[]> {
  return z.array(userTeamSchema).parse(await authedGetJson(`/admin/users/${userId}/teams`));
}

// ---- Manual tasks (Feature 5) ----

const manualTaskSchema = z.object({
  id: z.string(),
  user_id: z.string(),
  created_by: z.string().nullable(),
  title: z.string(),
  description: z.string(),
  status: z.string(),
  created_at: z.string(),
  updated_at: z.string(),
});
export type ManualTask = z.infer<typeof manualTaskSchema>;

/** An employee's manual tasks (`GET /admin/users/:id/tasks`). */
export async function fetchUserTasks(userId: string): Promise<ManualTask[]> {
  return z.array(manualTaskSchema).parse(await authedGetJson(`/admin/users/${userId}/tasks`));
}

/** Assign a task to an employee (`POST /admin/users/:id/tasks`). */
export async function createUserTask(
  userId: string,
  title: string,
  description: string,
): Promise<ManualTask> {
  return manualTaskSchema.parse(
    await authedJson("POST", `/admin/users/${userId}/tasks`, { title, description }),
  );
}

/** Update a task's status (`PATCH /admin/tasks/:id`). */
export async function setTaskStatus(taskId: string, status: string): Promise<ManualTask> {
  return manualTaskSchema.parse(await authedJson("PATCH", `/admin/tasks/${taskId}`, { status }));
}

/** Delete a task (`DELETE /admin/tasks/:id`). */
export async function deleteTask(taskId: string): Promise<void> {
  await authedJson("DELETE", `/admin/tasks/${taskId}`);
}

// ---- Candidate onboarding (Feature 6A) ----

const stageSchema = z.object({
  id: z.string(),
  name: z.string(),
  sequence: z.number(),
});
export type Stage = z.infer<typeof stageSchema>;

const candidateSchema = z.object({
  id: z.string(),
  name: z.string(),
  email: z.string(),
  position: z.string(),
  stage_id: z.string(),
  stage_name: z.string(),
  status: z.string(),
  converted_user_id: z.string().nullable(),
  hired_at: z.string().nullable(),
  created_at: z.string(),
  updated_at: z.string(),
});
export type Candidate = z.infer<typeof candidateSchema>;

const candidateTaskSchema = z.object({
  id: z.string(),
  candidate_id: z.string(),
  title: z.string(),
  done: z.boolean(),
  done_at: z.string().nullable(),
  created_at: z.string(),
});
export type CandidateTask = z.infer<typeof candidateTaskSchema>;

const candidateDocumentSchema = z.object({
  id: z.string(),
  doc_type: z.string(),
  storage_key: z.string(),
  created_at: z.string(),
  url: z.string(),
});
export type CandidateDocument = z.infer<typeof candidateDocumentSchema>;

const candidateDetailSchema = z.object({
  candidate: candidateSchema,
  tasks: z.array(candidateTaskSchema),
  documents: z.array(candidateDocumentSchema),
});
export type CandidateDetail = z.infer<typeof candidateDetailSchema>;

/** Pipeline stages (`GET /admin/onboarding/stages`). */
export async function fetchStages(): Promise<Stage[]> {
  return z.array(stageSchema).parse(await authedGetJson("/admin/onboarding/stages"));
}

/** All candidates for the Kanban board (`GET /admin/candidates`). */
export async function fetchCandidates(): Promise<Candidate[]> {
  return z.array(candidateSchema).parse(await authedGetJson("/admin/candidates"));
}

/** Create a candidate (`POST /admin/candidates`). */
export async function createCandidate(input: {
  name: string;
  email: string;
  position?: string;
  stage_id?: string;
}): Promise<Candidate> {
  return candidateSchema.parse(await authedJson("POST", "/admin/candidates", input));
}

/** Candidate detail with tasks + documents (`GET /admin/candidates/:id`). */
export async function fetchCandidate(id: string): Promise<CandidateDetail> {
  return candidateDetailSchema.parse(await authedGetJson(`/admin/candidates/${id}`));
}

/** Update fields / move stage / set status (`PATCH /admin/candidates/:id`). */
export async function updateCandidate(
  id: string,
  patch: {
    name?: string;
    email?: string;
    position?: string;
    stage_id?: string;
    status?: string;
  },
): Promise<Candidate> {
  return candidateSchema.parse(await authedJson("PATCH", `/admin/candidates/${id}`, patch));
}

/** Delete a candidate (`DELETE /admin/candidates/:id`). */
export async function deleteCandidate(id: string): Promise<void> {
  await authedJson("DELETE", `/admin/candidates/${id}`);
}

/** Add a checklist task (`POST /admin/candidates/:id/tasks`). */
export async function addCandidateTask(id: string, title: string): Promise<CandidateTask> {
  return candidateTaskSchema.parse(
    await authedJson("POST", `/admin/candidates/${id}/tasks`, { title }),
  );
}

/** Toggle a checklist task (`PATCH /admin/candidate-tasks/:tid`). */
export async function toggleCandidateTask(tid: string, done: boolean): Promise<void> {
  await authedJson("PATCH", `/admin/candidate-tasks/${tid}`, { done });
}

/** Delete a checklist task (`DELETE /admin/candidate-tasks/:tid`). */
export async function deleteCandidateTask(tid: string): Promise<void> {
  await authedJson("DELETE", `/admin/candidate-tasks/${tid}`);
}

/** Upload a document: presign → direct PUT to storage → save metadata.
 *  Bytes never pass through the API (Rule 5). */
export async function uploadCandidateDocument(
  id: string,
  file: File,
  docType: string,
): Promise<CandidateDocument> {
  const presign = (await authedJson("POST", `/admin/candidates/${id}/documents/presign`, {
    doc_type: docType,
    filename: file.name,
  })) as { url: string; storage_key: string };

  const put = await fetch(presign.url, { method: "PUT", body: file });
  if (!put.ok) {
    throw new Error(`Upload failed (status ${put.status}).`);
  }

  return candidateDocumentSchema.parse(
    await authedJson("POST", `/admin/candidates/${id}/documents`, {
      doc_type: docType,
      storage_key: presign.storage_key,
    }),
  );
}

/** Convert a candidate to an employee user (`POST /admin/candidates/:id/convert`).
 *  Returns the new user and the one-time temporary password. */
export async function convertCandidate(
  id: string,
): Promise<{ user: ManagedUser; password: string }> {
  const res = (await authedJson("POST", `/admin/candidates/${id}/convert`)) as {
    user: unknown;
    password: string;
  };
  return { user: userSummarySchema.parse(res.user), password: res.password };
}

// ---- Leave management (Feature 6B) ----

const leaveTypeSchema = z.object({
  id: z.string(),
  name: z.string(),
  paid: z.boolean(),
  default_days: z.number(),
});
export type LeaveType = z.infer<typeof leaveTypeSchema>;

const pendingLeaveSchema = z.object({
  id: z.string(),
  user_id: z.string(),
  employee_name: z.string(),
  employee_email: z.string(),
  leave_type_name: z.string(),
  start_date: z.string(),
  end_date: z.string(),
  days: z.number(),
  reason: z.string(),
  created_at: z.string(),
});
export type PendingLeave = z.infer<typeof pendingLeaveSchema>;

const holidaySchema = z.object({
  id: z.string(),
  day: z.string(),
  name: z.string(),
});
export type Holiday = z.infer<typeof holidaySchema>;

/** Pending leave requests the caller may act on (`GET /admin/leave/requests`).
 *  HR sees everyone; a project manager sees only their team. */
export async function fetchPendingLeave(): Promise<PendingLeave[]> {
  return z.array(pendingLeaveSchema).parse(await authedGetJson("/admin/leave/requests"));
}

/** Approve a request (`POST /admin/leave/requests/:id/approve`). */
export async function approveLeave(id: string): Promise<void> {
  await authedJson("POST", `/admin/leave/requests/${id}/approve`);
}

/** Reject a request (`POST /admin/leave/requests/:id/reject`). */
export async function rejectLeave(id: string): Promise<void> {
  await authedJson("POST", `/admin/leave/requests/${id}/reject`);
}

/** Leave types (`GET /me/leave/types` — readable by any authenticated user). */
export async function fetchLeaveTypes(): Promise<LeaveType[]> {
  return z.array(leaveTypeSchema).parse(await authedGetJson("/me/leave/types"));
}

/** Create a leave type (HR; `POST /admin/leave/types`). */
export async function createLeaveType(input: {
  name: string;
  paid: boolean;
  default_days: number;
}): Promise<LeaveType> {
  return leaveTypeSchema.parse(await authedJson("POST", "/admin/leave/types", input));
}

/** Allocate yearly days to an employee (HR; `POST /admin/leave/allocations`). */
export async function allocateLeave(input: {
  user_id: string;
  leave_type_id: string;
  year?: number;
  allotted_days: number;
}): Promise<void> {
  await authedJson("POST", "/admin/leave/allocations", input);
}

/** Company holidays (`GET /admin/holidays?year=`). */
export async function fetchHolidays(year?: number): Promise<Holiday[]> {
  const qs = year ? `?year=${year}` : "";
  return z.array(holidaySchema).parse(await authedGetJson(`/admin/holidays${qs}`));
}

/** Add a holiday (HR; `POST /admin/holidays`). */
export async function createHoliday(day: string, name: string): Promise<Holiday> {
  return holidaySchema.parse(await authedJson("POST", "/admin/holidays", { day, name }));
}

// ---- Attendance (Feature 6C) ----

const attendanceRowSchema = z.object({
  user_id: z.string(),
  name: z.string(),
  email: z.string(),
  present: z.number(),
  partial: z.number(),
  absent: z.number(),
  leave: z.number(),
  holiday: z.number(),
  weekend: z.number(),
  worked_seconds: z.number(),
});
export type AttendanceRow = z.infer<typeof attendanceRowSchema>;

const attendanceReportSchema = z.object({
  from: z.string(),
  to: z.string(),
  employees: z.array(attendanceRowSchema),
});
export type AttendanceReport = z.infer<typeof attendanceReportSchema>;

/** Per-employee attendance summary over a range (`GET /admin/attendance`).
 *  HR sees everyone; a project manager sees only their team. */
export async function fetchAttendanceReport(from: string, to: string): Promise<AttendanceReport> {
  return attendanceReportSchema.parse(
    await authedGetJson(`/admin/attendance?from=${from}&to=${to}`),
  );
}

/** Recompute a day's attendance for every employee (HR;
 *  `POST /admin/attendance/rollup?day=`). Defaults to yesterday. */
export async function rollupAttendance(day?: string): Promise<{ day: string; employees: number }> {
  const qs = day ? `?day=${day}` : "";
  return (await authedJson("POST", `/admin/attendance/rollup${qs}`)) as {
    day: string;
    employees: number;
  };
}

const attendanceDaySchema = z.object({
  user_id: z.string(),
  day: z.string(),
  status: z.string(),
  worked_seconds: z.number(),
  idle_seconds: z.number(),
  first_in_utc: z.string().nullable(),
  last_out_utc: z.string().nullable(),
  note: z.string(),
});
export type AttendanceDayRow = z.infer<typeof attendanceDaySchema>;

const attendanceCalendarSchema = z.object({
  from: z.string(),
  to: z.string(),
  days: z.array(attendanceDaySchema),
});
export type AttendanceCalendar = z.infer<typeof attendanceCalendarSchema>;

/** One employee's attendance calendar (`GET /admin/users/:id/attendance`). */
export async function fetchUserAttendance(
  userId: string,
  from: string,
  to: string,
): Promise<AttendanceCalendar> {
  return attendanceCalendarSchema.parse(
    await authedGetJson(`/admin/users/${userId}/attendance?from=${from}&to=${to}`),
  );
}

const segmentSchema = z.object({
  start_utc: z.string(),
  end_utc: z.string(),
  kind: z.enum(["active", "idle", "meeting", "break"]),
});
export type TimelineSegment = z.infer<typeof segmentSchema>;

const timelineSchema = z.object({
  from: z.string(),
  to: z.string(),
  segments: z.array(segmentSchema),
});
export type DayTimeline = z.infer<typeof timelineSchema>;

/** Activity segments for an employee's day (`GET /admin/users/:id/timeline`). */
export async function fetchUserTimeline(
  userId: string,
  fromIso: string,
  toIso: string,
): Promise<DayTimeline> {
  const qs = `from=${encodeURIComponent(fromIso)}&to=${encodeURIComponent(toIso)}`;
  return timelineSchema.parse(await authedGetJson(`/admin/users/${userId}/timeline?${qs}`));
}

// ---- User management (HR) ----

const userSummarySchema = z.object({
  id: z.string(),
  name: z.string(),
  email: z.string(),
  role: roleSchema,
  manager_id: z.string().nullable(),
  team_id: z.string().nullable(),
  created_at: z.string(),
});
export type ManagedUser = z.infer<typeof userSummarySchema>;

export type NewUser = {
  name: string;
  email: string;
  password: string;
  role: Role;
  manager_id?: string | null;
};

export async function listUsers(): Promise<ManagedUser[]> {
  return z.array(userSummarySchema).parse(await authedGetJson("/admin/users"));
}

export async function createUser(u: NewUser): Promise<ManagedUser> {
  return userSummarySchema.parse(await authedJson("POST", "/admin/users", u));
}

export async function deleteUser(id: string): Promise<void> {
  await authedJson("DELETE", `/admin/users/${id}`);
}

/** Reset a user's password (HR). Returns the new password to hand over once.
 *  Pass a password to set a specific one, or omit to auto-generate. */
export async function resetPassword(id: string, password?: string): Promise<string> {
  const res = (await authedJson(
    "POST",
    `/admin/users/${id}/reset-password`,
    password ? { password } : {},
  )) as { password: string };
  return res.password;
}

/** `POST /auth/login`. */
export async function login(email: string, password: string): Promise<LoginResponse> {
  const res = await fetch(`${API_BASE}/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  if (res.status === 401) {
    throw new Error("Invalid email or password.");
  }
  if (!res.ok) {
    throw new Error(`Login failed (status ${res.status}).`);
  }
  return loginResponseSchema.parse(await res.json());
}
