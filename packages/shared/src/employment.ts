import { z } from "zod";

/**
 * Employment type — an HR classification of a worker (employee, contractor, or
 * intern), separate from the RBAC role. Mirrors the Rust `EmploymentType` enum
 * and the Postgres `employment_type` type — never use magic strings (CLAUDE.md).
 */
export const EMPLOYMENT_TYPES = ["employee", "contractor", "intern"] as const;

export const employmentTypeSchema = z.enum(EMPLOYMENT_TYPES);

export type EmploymentType = z.infer<typeof employmentTypeSchema>;

/** Human-readable labels for display. */
export const EMPLOYMENT_TYPE_LABEL: Record<EmploymentType, string> = {
  employee: "Employee",
  contractor: "Contractor",
  intern: "Intern",
};
