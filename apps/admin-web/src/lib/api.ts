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
