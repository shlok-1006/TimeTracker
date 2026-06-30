import type { PresenceStatus } from "@/lib/api";

/** Spec colors: green=working, yellow=idle, blue=break, red=not logged in. */
export const STATUS_STYLES: Record<PresenceStatus, { label: string; dot: string; text: string }> = {
  working: { label: "Working", dot: "bg-green-500", text: "text-green-700" },
  idle: { label: "Idle", dot: "bg-yellow-500", text: "text-yellow-700" },
  break: { label: "Break", dot: "bg-blue-500", text: "text-blue-700" },
  meeting: { label: "In meeting", dot: "bg-purple-500", text: "text-purple-700" },
  not_working: { label: "Day ended", dot: "bg-slate-400", text: "text-slate-600" },
  not_logged_in: { label: "Not logged in", dot: "bg-red-500", text: "text-red-700" },
};
