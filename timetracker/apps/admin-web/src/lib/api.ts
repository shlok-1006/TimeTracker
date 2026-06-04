import { z } from "zod";
import { roleSchema } from "@timetracker/shared";

const API_BASE = process.env.NEXT_PUBLIC_API_BASE_URL ?? "http://localhost:8090";

const loginResponseSchema = z.object({
  access_token: z.string(),
  token_type: z.string(),
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

function authHeaders(token: string) {
  return { Authorization: `Bearer ${token}` };
}

async function getJson(path: string, token: string) {
  const res = await fetch(`${API_BASE}${path}`, { headers: authHeaders(token) });
  if (res.status === 401 || res.status === 403) {
    throw new Error("Not authorized.");
  }
  if (!res.ok) {
    throw new Error(`Request failed (status ${res.status}).`);
  }
  return res.json();
}

/** Fetch the live team roster (`GET /admin/team`). */
export async function fetchTeam(token: string): Promise<TeamMember[]> {
  return teamSchema.parse(await getJson("/admin/team", token));
}

const hoursSummarySchema = z.object({
  total_seconds: z.number(),
  today_seconds: z.number(),
  week_seconds: z.number(),
  active_seconds: z.number(),
  idle_seconds: z.number(),
});
export type HoursSummary = z.infer<typeof hoursSummarySchema>;

/** Drill-down hours for one employee (`GET /admin/users/:id/hours`). */
export async function fetchUserHours(token: string, userId: string): Promise<HoursSummary> {
  return hoursSummarySchema.parse(await getJson(`/admin/users/${userId}/hours`, token));
}

const adminShotSchema = z.object({
  id: z.string(),
  taken_at: z.string(),
  url: z.string(),
});
export type AdminShot = z.infer<typeof adminShotSchema>;

/** Drill-down screenshots for one employee (`GET /admin/users/:id/screenshots`). */
export async function fetchUserScreenshots(token: string, userId: string): Promise<AdminShot[]> {
  return z.array(adminShotSchema).parse(await getJson(`/admin/users/${userId}/screenshots`, token));
}

/** Call `POST /auth/login` and validate the response shape. */
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
