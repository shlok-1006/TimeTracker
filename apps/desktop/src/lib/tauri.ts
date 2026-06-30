/** Lazily import the Tauri `invoke` (so the app still loads in a plain browser). */
export async function invoker() {
  return (await import("@tauri-apps/api/core")).invoke;
}

/** Format seconds as "Hh Mm Ss". */
export function fmtHms(total: number) {
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = Math.floor(total % 60);
  return `${h}h ${m}m ${s}s`;
}

export const STATUS_LABEL: Record<string, { label: string; dot: string }> = {
  working: { label: "Working", dot: "bg-green-500" },
  idle: { label: "Idle", dot: "bg-amber-500" },
  break: { label: "On break", dot: "bg-blue-500" },
  meeting: { label: "In meeting", dot: "bg-purple-500" },
  not_working: { label: "Day ended", dot: "bg-slate-400" },
};

export type EmployeeSession = {
  id: string;
  name: string;
  email: string;
  role: string;
};
