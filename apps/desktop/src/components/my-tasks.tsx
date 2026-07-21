"use client";

import { useQuery } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

type Task = {
  id: string;
  title: string;
  description: string;
  status: string;
  weight: number;
  due_date: string | null;
  created_at: string;
  updated_at: string;
};

/** "2026-07-25" → "Jul 25, 2026" (dates are calendar days, no timezone). */
function fmtDue(day: string): string {
  const [y, m, d] = day.split("-").map(Number);
  if (!y || !m || !d) return day;
  return new Date(y, m - 1, d).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/** A due date is overdue if it's before today and the task is still open. */
function isOverdue(day: string, status: string): boolean {
  if (status === "done") return false;
  const today = new Date();
  const todayStr = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(
    today.getDate(),
  ).padStart(2, "0")}`;
  return day < todayStr;
}

/** HR/PM-assigned tasks shown on the employee dashboard (read-only). These are
 *  analysed by the AI like tickets, but never appear in Linear. */
export function MyTasks() {
  const tasks = useQuery({
    queryKey: ["me_tasks"],
    queryFn: async () => (await invoker())<Task[]>("me_tasks"),
    refetchInterval: 60_000,
  });

  return (
    <section className="flex flex-col gap-3 rounded-lg border border-slate-200 p-6 dark:border-slate-800">
      <h2 className="font-semibold">Assigned tasks</h2>

      {tasks.isLoading && <p className="text-sm text-slate-500">Loading…</p>}
      {tasks.error && (
        <p className="text-sm text-red-600">
          {tasks.error instanceof Error ? tasks.error.message : String(tasks.error)}
        </p>
      )}
      {tasks.data && tasks.data.length === 0 && (
        <p className="rounded-md bg-slate-50 p-3 text-sm text-slate-500 dark:bg-slate-800/40">
          No tasks assigned to you.
        </p>
      )}

      <ul className="flex flex-col gap-2">
        {tasks.data?.map((t) => (
          <li
            key={t.id}
            className="flex items-start justify-between gap-3 rounded-md border border-slate-200 p-3 dark:border-slate-700"
          >
            <div className={t.status === "done" ? "opacity-60" : ""}>
              <p className={`font-medium ${t.status === "done" ? "line-through" : ""}`}>
                {t.title}
              </p>
              {t.description && (
                <p className="text-sm text-slate-500">{t.description}</p>
              )}
              <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-slate-500">
                <span className="rounded bg-slate-100 px-1.5 py-0.5 font-medium text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                  Weight {t.weight}/10
                </span>
                {t.due_date && (
                  <span
                    className={
                      isOverdue(t.due_date, t.status)
                        ? "font-medium text-red-600"
                        : "text-slate-500"
                    }
                  >
                    Due {fmtDue(t.due_date)}
                    {isOverdue(t.due_date, t.status) ? " · overdue" : ""}
                  </span>
                )}
              </div>
            </div>
            <span
              className={`shrink-0 rounded-full px-2 py-0.5 text-[11px] font-medium ${
                t.status === "done"
                  ? "bg-green-100 text-green-800"
                  : "bg-amber-100 text-amber-800"
              }`}
            >
              {t.status}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
