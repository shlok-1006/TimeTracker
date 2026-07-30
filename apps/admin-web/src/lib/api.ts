import { z } from "zod";
import {
  roleSchema,
  type Role,
  employmentTypeSchema,
  type EmploymentType,
} from "@timetracker/shared";
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

// A "day's work" = active + idle + meeting (only Break is excluded). today/week
// are period-scoped totals of that, each broken out so idle and meeting show
// on their own. total_seconds is all-time worked (reconcile line only).
const hoursSummarySchema = z.object({
  today_seconds: z.number(),
  today_active_seconds: z.number(),
  today_idle_seconds: z.number(),
  today_meeting_seconds: z.number(),
  week_seconds: z.number(),
  week_active_seconds: z.number(),
  week_idle_seconds: z.number(),
  week_meeting_seconds: z.number(),
  week_grace_seconds: z.number(),
  total_seconds: z.number(),
});
export type HoursSummary = z.infer<typeof hoursSummarySchema>;

// ---- Manual "grace" time grants (HR / PM) ----

const timeGrantSchema = z.object({
  id: z.string(),
  user_id: z.string(),
  week_start: z.string(),
  seconds: z.number(),
  reason: z.string(),
  granted_by: z.string().nullable(),
  granted_by_name: z.string().nullable(),
  created_at: z.string(),
});
export type TimeGrant = z.infer<typeof timeGrantSchema>;

/** This week's grace grants for a user (`GET /admin/users/:id/time-grants`). */
export async function fetchTimeGrants(
  userId: string,
): Promise<{ week_start: string; grants: TimeGrant[] }> {
  return z
    .object({ week_start: z.string(), grants: z.array(timeGrantSchema) })
    .parse(await authedGetJson(`/admin/users/${userId}/time-grants`));
}

/** Add grace time to a user's current week (`POST /admin/users/:id/time-grants`). */
export async function addTimeGrant(
  userId: string,
  input: { hours: number; minutes: number; reason: string },
): Promise<TimeGrant> {
  return timeGrantSchema.parse(
    await authedJson("POST", `/admin/users/${userId}/time-grants`, input),
  );
}

/** Remove a grace grant (`DELETE /admin/time-grants/:id`). */
export async function deleteTimeGrant(id: string): Promise<void> {
  await authedJson("DELETE", `/admin/time-grants/${id}`);
}

const adminShotSchema = z.object({
  id: z.string(),
  taken_at: z.string(),
  url: z.string(),
});
export type AdminShot = z.infer<typeof adminShotSchema>;

/** Single-flight guard: concurrent 401s must NOT each rotate the refresh token.
 *  The server rotates on the first use and its reuse-detection treats the second
 *  use of the same token as a stolen-token replay — revoking every session and
 *  logging the user out. While one rotation is in flight, all callers await the
 *  same promise and reuse its result. */
let refreshInFlight: Promise<boolean> | null = null;

/** Try to rotate the refresh token. Returns true if a new access token is set.
 *  De-duplicated across concurrent callers via `refreshInFlight`. */
function tryRefresh(): Promise<boolean> {
  if (!refreshInFlight) {
    refreshInFlight = doRefresh().finally(() => {
      refreshInFlight = null;
    });
  }
  return refreshInFlight;
}

async function doRefresh(): Promise<boolean> {
  // Start from the freshest token — another tab may have rotated it since we
  // last touched in-memory state. localStorage is the cross-tab source of truth.
  const rt = useAuthStore.getState().adoptFromStorage();
  if (!rt) return false;
  try {
    const res = await fetch(`${API_BASE}/auth/refresh`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ refresh_token: rt }),
    });
    if (res.status === 401) {
      // The token was rejected. If a sibling tab rotated in the meantime, adopt
      // its token and let the caller retry instead of logging every tab out.
      const latest = useAuthStore.getState().adoptFromStorage();
      if (latest && latest !== rt) return true;
      useAuthStore.getState().clear();
      return false;
    }
    if (!res.ok) return false; // transient (5xx/network) — keep the session
    const data = (await res.json()) as { access_token: string; refresh_token: string };
    useAuthStore.getState().setTokens(data.access_token, data.refresh_token);
    return true;
  } catch {
    return false; // network error — keep the session, don't force a re-login
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

// ---- Range analysis: verify every screenshot in a time window ----

const rangePreviewSchema = z.object({
  from: z.string(),
  to: z.string(),
  total: z.number(),
  analyzable: z.number(),
  cap: z.number(),
  claude_configured: z.boolean(),
  model: z.string(),
});
export type RangePreview = z.infer<typeof rangePreviewSchema>;

/** Count the screenshots in a window before committing to analyze them
 *  (`GET /admin/users/:id/analyze-range/preview?from=&to=`). ISO timestamps. */
export async function previewAnalyzeRange(
  userId: string,
  fromIso: string,
  toIso: string,
): Promise<RangePreview> {
  const qs = `from=${encodeURIComponent(fromIso)}&to=${encodeURIComponent(toIso)}`;
  return rangePreviewSchema.parse(
    await authedGetJson(`/admin/users/${userId}/analyze-range/preview?${qs}`),
  );
}

const rangeStartSchema = z.object({
  run_id: z.string(),
  total: z.number(),
  model: z.string(),
});

/** Start analyzing EVERY working screenshot in the window
 *  (`POST /admin/users/:id/analyze-range?from=&to=`). Returns a run id to poll. */
export async function startAnalyzeRange(
  userId: string,
  fromIso: string,
  toIso: string,
): Promise<{ run_id: string; total: number; model: string }> {
  const qs = `from=${encodeURIComponent(fromIso)}&to=${encodeURIComponent(toIso)}`;
  return rangeStartSchema.parse(
    await authedJson("POST", `/admin/users/${userId}/analyze-range?${qs}`),
  );
}

const analysisRunSchema = z.object({
  id: z.string(),
  user_id: z.string(),
  from_utc: z.string(),
  to_utc: z.string(),
  status: z.enum(["running", "completed", "failed"]),
  total: z.number(),
  analyzed: z.number(),
  skipped: z.number(),
  failed: z.number(),
  error: z.string().nullable(),
  created_at: z.string(),
  finished_at: z.string().nullable(),
});
export type AnalysisRun = z.infer<typeof analysisRunSchema>;

/** Live progress of a range run (`GET /admin/analysis-runs/:id`). */
export async function fetchAnalysisRun(runId: string): Promise<AnalysisRun> {
  return analysisRunSchema.parse(await authedGetJson(`/admin/analysis-runs/${runId}`));
}

// ---- Manager assignment (multi-manager; HR only) ----

const managerSchema = z.object({ id: z.string(), name: z.string(), email: z.string() });
export type Manager = z.infer<typeof managerSchema>;

/** A user's assigned managers (`GET /admin/users/:id/managers`). */
export async function fetchUserManagers(userId: string): Promise<Manager[]> {
  return z.array(managerSchema).parse(await authedGetJson(`/admin/users/${userId}/managers`));
}

/** Replace a user's manager set — any number of PMs, or none
 *  (`PUT /admin/users/:id/managers`). Returns the new set. */
export async function setUserManagers(userId: string, managerIds: string[]): Promise<Manager[]> {
  return z
    .array(managerSchema)
    .parse(await authedJson("PUT", `/admin/users/${userId}/managers`, { manager_ids: managerIds }));
}

// ---- Activity (app usage + input-activity levels) ----

const activitySchema = z.object({
  day: z.string(),
  activity_pct: z.number().nullable(),
  apps: z.array(z.object({ app_name: z.string(), seconds: z.number() })),
  blocks: z.array(
    z.object({
      block_start: z.string(),
      active_seconds: z.number(),
      total_seconds: z.number(),
    }),
  ),
});
export type UserActivity = z.infer<typeof activitySchema>;

/** One employee's activity breakdown for a day (`GET /admin/users/:id/activity?day=`). */
export async function fetchUserActivity(userId: string, day: string): Promise<UserActivity> {
  return activitySchema.parse(
    await authedGetJson(`/admin/users/${userId}/activity?day=${day}`),
  );
}

// ---- OKF policy library (HR edits; everyone reads) ----

const policySummarySchema = z.object({
  id: z.string(),
  slug: z.string(),
  title: z.string(),
  category: z.string(),
  kind: z.enum(["markdown", "file"]),
  file_name: z.string().nullable(),
  updated_at: z.string(),
});
export type PolicySummary = z.infer<typeof policySummarySchema>;

const policyDocSchema = z.object({
  id: z.string(),
  slug: z.string(),
  title: z.string(),
  category: z.string(),
  kind: z.enum(["markdown", "file"]),
  content: z.string(),
  storage_key: z.string().nullable(),
  file_name: z.string().nullable(),
  content_type: z.string().nullable(),
  size_bytes: z.number().nullable(),
  sort_order: z.number(),
  updated_by: z.string().nullable(),
  updated_by_name: z.string().nullable(),
  updated_at: z.string(),
});
export type PolicyDoc = z.infer<typeof policyDocSchema>;

/** All policy documents (`GET /policies`, any signed-in user). */
export async function listPolicies(): Promise<PolicySummary[]> {
  return z.array(policySummarySchema).parse(await authedGetJson("/policies"));
}

/** One policy document (`GET /policies/:id`). */
export async function getPolicy(id: string): Promise<PolicyDoc> {
  return policyDocSchema.parse(await authedGetJson(`/policies/${id}`));
}

/** Create a markdown document (`POST /admin/policies`, HR only). */
export async function createPolicy(input: {
  title: string;
  category: string;
  content: string;
}): Promise<PolicyDoc> {
  return policyDocSchema.parse(await authedJson("POST", "/admin/policies", input));
}

/** Edit a document (`PUT /admin/policies/:id`, HR only). */
export async function updatePolicy(
  id: string,
  input: { title: string; category: string; content: string },
): Promise<PolicyDoc> {
  return policyDocSchema.parse(await authedJson("PUT", `/admin/policies/${id}`, input));
}

/** Delete a document (`DELETE /admin/policies/:id`, HR only). */
export async function deletePolicy(id: string): Promise<void> {
  await authedJson("DELETE", `/admin/policies/${id}`);
}

/** Presign a PUT to upload a file attachment (`POST /admin/policies/upload-url`). */
export async function getPolicyUploadUrl(
  fileName: string,
): Promise<{ url: string; storage_key: string }> {
  return z
    .object({ url: z.string(), storage_key: z.string() })
    .parse(await authedJson("POST", "/admin/policies/upload-url", { file_name: fileName }));
}

/** Register an uploaded file as a document (`POST /admin/policies/file`, HR only). */
export async function createFilePolicy(input: {
  title: string;
  category: string;
  storage_key: string;
  file_name: string;
  content_type: string;
  size_bytes: number;
}): Promise<PolicyDoc> {
  return policyDocSchema.parse(await authedJson("POST", "/admin/policies/file", input));
}

/** A short-lived download URL for a file document (`GET /policies/:id/download`). */
export async function getPolicyDownloadUrl(
  id: string,
): Promise<{ url: string; file_name: string | null }> {
  return z
    .object({ url: z.string(), file_name: z.string().nullable() })
    .parse(await authedGetJson(`/policies/${id}/download`));
}

/** Upload a file end-to-end: presign → PUT to storage → register the document. */
export async function uploadPolicyFile(file: File, category: string): Promise<PolicyDoc> {
  const { url, storage_key } = await getPolicyUploadUrl(file.name);
  const put = await fetch(url, {
    method: "PUT",
    body: file,
    headers: { "content-type": file.type || "application/octet-stream" },
  });
  if (!put.ok) throw new Error(`upload failed (status ${put.status})`);
  return createFilePolicy({
    title: file.name,
    category,
    storage_key,
    file_name: file.name,
    content_type: file.type || "application/octet-stream",
    size_bytes: file.size,
  });
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
  weight: z.number(),
  due_date: z.string().nullable(),
  created_at: z.string(),
  updated_at: z.string(),
});
export type ManualTask = z.infer<typeof manualTaskSchema>;

/** An employee's manual tasks (`GET /admin/users/:id/tasks`). */
export async function fetchUserTasks(userId: string): Promise<ManualTask[]> {
  return z.array(manualTaskSchema).parse(await authedGetJson(`/admin/users/${userId}/tasks`));
}

/** Assign a task to an employee (`POST /admin/users/:id/tasks`).
 *  `weight` is 1–10; `dueDate` is "YYYY-MM-DD" or null (open-ended). */
export async function createUserTask(
  userId: string,
  title: string,
  description: string,
  weight: number,
  dueDate: string | null,
): Promise<ManualTask> {
  return manualTaskSchema.parse(
    await authedJson("POST", `/admin/users/${userId}/tasks`, {
      title,
      description,
      weight,
      due_date: dueDate,
    }),
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
  default_days_contractor: z.number(),
  default_days_intern: z.number(),
});
export type LeaveType = z.infer<typeof leaveTypeSchema>;

const leaveBalanceSchema = z.object({
  leave_type_id: z.string(),
  leave_type_name: z.string(),
  paid: z.boolean(),
  allotted_days: z.number(),
  used_days: z.number(),
  remaining_days: z.number(),
  is_override: z.boolean(),
});
export type LeaveBalance = z.infer<typeof leaveBalanceSchema>;

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

/** Create a leave type with per-category defaults (HR; `POST /admin/leave/types`). */
export async function createLeaveType(input: {
  name: string;
  paid: boolean;
  default_days: number;
  default_days_contractor: number;
  default_days_intern: number;
}): Promise<LeaveType> {
  return leaveTypeSchema.parse(await authedJson("POST", "/admin/leave/types", input));
}

/** Update a leave type's paid flag + per-category defaults (HR; `PATCH /admin/leave/types/:id`). */
export async function updateLeaveType(
  id: string,
  input: {
    paid: boolean;
    default_days: number;
    default_days_contractor: number;
    default_days_intern: number;
  },
): Promise<LeaveType> {
  return leaveTypeSchema.parse(await authedJson("PATCH", `/admin/leave/types/${id}`, input));
}

/** Set a user's yearly allotment (override) for a type (HR; `POST /admin/leave/allocations`). */
export async function allocateLeave(input: {
  user_id: string;
  leave_type_id: string;
  year?: number;
  allotted_days: number;
}): Promise<void> {
  await authedJson("POST", "/admin/leave/allocations", input);
}

/** Increase/decrease a user's allotment by a delta (HR; `POST /admin/leave/allocations/adjust`). */
export async function adjustLeaveAllocation(input: {
  user_id: string;
  leave_type_id: string;
  year?: number;
  delta: number;
}): Promise<void> {
  await authedJson("POST", "/admin/leave/allocations/adjust", input);
}

/** Remove a user's override, reverting to the category default (HR; `DELETE /admin/leave/allocations`). */
export async function deleteLeaveAllocation(input: {
  user_id: string;
  leave_type_id: string;
  year?: number;
}): Promise<void> {
  await authedJson("DELETE", "/admin/leave/allocations", input);
}

/** A user's per-type balances for the allocation UI (HR; `GET /admin/users/:id/leave/balance`). */
export async function fetchUserLeaveBalance(
  userId: string,
  year?: number,
): Promise<{ year: number; balances: LeaveBalance[] }> {
  const qs = year ? `?year=${year}` : "";
  return z
    .object({ year: z.number(), balances: z.array(leaveBalanceSchema) })
    .parse(await authedGetJson(`/admin/users/${userId}/leave/balance${qs}`));
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
  is_override: z.boolean(),
});
export type AttendanceDayRow = z.infer<typeof attendanceDaySchema>;

/** The attendance statuses HR may assign (must match the server CHECK). */
export const ATTENDANCE_STATUSES = [
  "present",
  "partial",
  "absent",
  "leave",
  "holiday",
  "weekend",
] as const;
export type AttendanceStatus = (typeof ATTENDANCE_STATUSES)[number];

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

/** HR: set (override) a user's attendance status for a day
 *  (`PUT /admin/users/:id/attendance/:day`). */
export async function setUserAttendance(
  userId: string,
  day: string,
  status: AttendanceStatus,
  note: string,
): Promise<AttendanceDayRow> {
  return attendanceDaySchema.parse(
    await authedJson("PUT", `/admin/users/${userId}/attendance/${day}`, { status, note }),
  );
}

/** HR: revert a day to the auto-derived status
 *  (`DELETE /admin/users/:id/attendance/:day`). */
export async function clearUserAttendance(
  userId: string,
  day: string,
): Promise<AttendanceDayRow> {
  return attendanceDaySchema.parse(
    await authedJson("DELETE", `/admin/users/${userId}/attendance/${day}`),
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
  employment_type: employmentTypeSchema,
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
  employment_type: EmploymentType;
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

/** Set a user's employment type (`PUT /admin/users/:id/employment-type`, HR). */
export async function setUserEmploymentType(
  id: string,
  employmentType: EmploymentType,
): Promise<void> {
  await authedJson("PUT", `/admin/users/${id}/employment-type`, {
    employment_type: employmentType,
  });
}

/** A former employee, retained after removal (Alumni section). */
const alumnusSchema = z.object({
  id: z.string(),
  user_id: z.string().nullable(),
  name: z.string(),
  email: z.string(),
  role: roleSchema,
  team_id: z.string().nullable(),
  joined_at: z.string().nullable(),
  removed_at: z.string(),
  removed_by: z.string().nullable(),
});
export type Alumnus = z.infer<typeof alumnusSchema>;

/** Removed employees, most recently removed first (`GET /admin/alumni`, HR). */
export async function listAlumni(): Promise<Alumnus[]> {
  return z.array(alumnusSchema).parse(await authedGetJson("/admin/alumni"));
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

/** Change the password from the login screen (verifying the current one) and
 *  sign in with the new one. Public — no token required (`POST /auth/change-password`). */
export async function changePassword(
  email: string,
  currentPassword: string,
  newPassword: string,
): Promise<LoginResponse> {
  const res = await fetch(`${API_BASE}/auth/change-password`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      email,
      current_password: currentPassword,
      new_password: newPassword,
    }),
  });
  if (res.status === 401) {
    throw new Error("Current email or password is incorrect.");
  }
  if (res.status === 400) {
    // Surface the server's validation message (e.g. min length).
    let msg = "Invalid new password.";
    try {
      const j = (await res.json()) as { error?: string };
      if (j.error) msg = j.error;
    } catch {
      /* keep default */
    }
    throw new Error(msg);
  }
  if (!res.ok) {
    throw new Error(`Password change failed (status ${res.status}).`);
  }
  return loginResponseSchema.parse(await res.json());
}
