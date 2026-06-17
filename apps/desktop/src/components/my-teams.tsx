"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

type Team = { id: string; name: string; description: string; created_at: string };

/** Self-service team selection. Toggling a team joins/leaves it; the choice
 *  feeds the pre-timer dropdown and the admin's team summary + per-employee view. */
export function MyTeams() {
  const qc = useQueryClient();

  const options = useQuery({
    queryKey: ["me_team_options"],
    queryFn: async () => (await invoker())<Team[]>("me_team_options"),
  });
  const mine = useQuery({
    queryKey: ["me_teams"],
    queryFn: async () => (await invoker())<Team[]>("me_teams"),
  });

  const memberIds = new Set((mine.data ?? []).map((t) => t.id));

  const toggle = useMutation({
    mutationFn: async ({ id, isMember }: { id: string; isMember: boolean }) => {
      const invoke = await invoker();
      await invoke(isMember ? "leave_team" : "join_team", { teamId: id });
    },
    // Refresh memberships → updates the chips and the pre-timer dropdown.
    onSuccess: () => qc.invalidateQueries({ queryKey: ["me_teams"] }),
  });

  return (
    <section className="flex flex-col gap-3 rounded-lg border border-slate-200 p-6 dark:border-slate-800">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">My teams</h2>
        <span className="text-xs text-slate-500">Select the team(s) you work with</span>
      </div>

      {options.isLoading && <p className="text-sm text-slate-500">Loading…</p>}
      {options.error && (
        <p className="text-sm text-red-600">
          Couldn&apos;t load teams:{" "}
          {options.error instanceof Error ? options.error.message : String(options.error)}
        </p>
      )}

      <div className="flex flex-wrap gap-2">
        {options.data?.map((t) => {
          const isMember = memberIds.has(t.id);
          return (
            <button
              key={t.id}
              disabled={toggle.isPending}
              onClick={() => toggle.mutate({ id: t.id, isMember })}
              className={`rounded-full border px-3 py-1.5 text-sm font-medium transition disabled:opacity-50 ${
                isMember
                  ? "border-green-600 bg-green-600 text-white hover:bg-green-700"
                  : "border-slate-300 bg-white text-slate-700 hover:border-slate-400 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"
              }`}
            >
              {isMember ? "✓ " : ""}
              {t.name}
            </button>
          );
        })}
      </div>

      {!options.isLoading && memberIds.size === 0 && (
        <p className="text-xs text-slate-500">
          You&apos;re not in any team yet. Select one above — you&apos;ll then choose it before
          starting the timer.
        </p>
      )}
    </section>
  );
}
