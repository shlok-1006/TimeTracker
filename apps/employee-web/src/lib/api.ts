import { z } from "zod";
import { roleSchema } from "@timetracker/shared";
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

// ---- Hours (own) ----

const hoursSummarySchema = z.object({
  total_seconds: z.number(),
  today_seconds: z.number(),
  week_seconds: z.number(),
  active_seconds: z.number(),
  idle_seconds: z.number(),
});
export type HoursSummary = z.infer<typeof hoursSummarySchema>;

/** `GET /me/hours`. */
export async function fetchMyHours(): Promise<HoursSummary> {
  return hoursSummarySchema.parse(await authedGetJson("/me/hours"));
}

// ---- Daily AI report (own) ----

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

/** `GET /me/report?day=`. */
export async function fetchMyReport(day: string): Promise<DailyReport | null> {
  const res = (await authedGetJson(`/me/report?day=${day}`)) as { report: unknown };
  return res.report ? dailyReportSchema.parse(res.report) : null;
}

// ---- Screenshots (own) ----

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

/** `GET /me/screenshots?day=`. */
export async function fetchMyDayScreenshots(day: string): Promise<DayShot[]> {
  return z.array(dayShotSchema).parse(await authedGetJson(`/me/screenshots?day=${day}`));
}

// ---- Leave (own) ----

const leaveTypeSchema = z.object({
  id: z.string(),
  name: z.string(),
  paid: z.boolean(),
  default_days: z.number(),
});
export type LeaveType = z.infer<typeof leaveTypeSchema>;

const balanceSchema = z.object({
  leave_type_id: z.string(),
  leave_type_name: z.string(),
  paid: z.boolean(),
  allotted_days: z.number(),
  used_days: z.number(),
  remaining_days: z.number(),
});
export type LeaveBalance = z.infer<typeof balanceSchema>;

const balanceRespSchema = z.object({
  year: z.number(),
  balances: z.array(balanceSchema),
});
export type LeaveBalanceResp = z.infer<typeof balanceRespSchema>;

const leaveRequestSchema = z.object({
  id: z.string(),
  leave_type_name: z.string(),
  start_date: z.string(),
  end_date: z.string(),
  days: z.number(),
  reason: z.string(),
  status: z.enum(["pending", "approved", "rejected", "cancelled"]),
  created_at: z.string(),
});
export type LeaveRequest = z.infer<typeof leaveRequestSchema>;

/** `GET /me/leave/types` — readable by any authenticated user. */
export async function fetchLeaveTypes(): Promise<LeaveType[]> {
  return z.array(leaveTypeSchema).parse(await authedGetJson("/me/leave/types"));
}

/** `GET /me/leave/balance?year=` (defaults to current year). */
export async function fetchMyLeaveBalance(year?: number): Promise<LeaveBalanceResp> {
  const qs = year ? `?year=${year}` : "";
  return balanceRespSchema.parse(await authedGetJson(`/me/leave/balance${qs}`));
}

/** `GET /me/leave/requests`. */
export async function fetchMyLeaveRequests(): Promise<LeaveRequest[]> {
  return z.array(leaveRequestSchema).parse(await authedGetJson("/me/leave/requests"));
}

/** `POST /me/leave/requests`. */
export async function requestLeave(input: {
  leave_type_id: string;
  start_date: string;
  end_date: string;
  reason: string;
}): Promise<{ id: string; days: number; status: string }> {
  return (await authedJson("POST", "/me/leave/requests", input)) as {
    id: string;
    days: number;
    status: string;
  };
}

/** `POST /me/leave/requests/:id/cancel`. */
export async function cancelLeaveRequest(id: string): Promise<void> {
  await authedJson("POST", `/me/leave/requests/${id}/cancel`);
}

// ---- Attendance (own) ----

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

/** `GET /me/attendance?from=&to=`. */
export async function fetchMyAttendance(from: string, to: string): Promise<AttendanceCalendar> {
  return attendanceCalendarSchema.parse(await authedGetJson(`/me/attendance?from=${from}&to=${to}`));
}
