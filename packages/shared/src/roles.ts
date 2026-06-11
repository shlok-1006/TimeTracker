import { z } from "zod";

/**
 * The three system roles. This is the single source of truth shared by the
 * desktop app and the admin dashboard, mirroring the Rust `UserRole` enum and
 * the Postgres `user_role` type — never use magic strings (CLAUDE.md).
 */
export const ROLES = ["employee", "project_manager", "hr"] as const;

export const roleSchema = z.enum(ROLES);

export type Role = z.infer<typeof roleSchema>;

/**
 * Privilege ranking. `employee` uses the desktop app; `project_manager` and
 * `hr` are the admin-dashboard roles, with `hr` holding the highest privilege.
 */
export const ROLE_RANK: Record<Role, number> = {
  employee: 0,
  project_manager: 1,
  hr: 2,
};

/** True if `role` meets or exceeds the `required` role. */
export function roleAtLeast(role: Role, required: Role): boolean {
  return ROLE_RANK[role] >= ROLE_RANK[required];
}
