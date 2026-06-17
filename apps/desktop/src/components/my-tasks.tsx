"use client";

import { useQuery } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

type Task = {
  id: string;
  title: string;
  description: string;
  status: string;
  created_at: string;
  updated_at: string;
};

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
