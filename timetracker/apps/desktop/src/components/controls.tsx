"use client";

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoker, STATUS_LABEL } from "@/lib/tauri";

/** Tracking controls: start/stop, break, meeting mode + live status badge. */
export function Controls({ userId }: { userId: string }) {
  const qc = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const { data: tracking } = useQuery({
    queryKey: ["is_tracking"],
    queryFn: async () => (await invoker())<boolean>("is_tracking"),
    refetchInterval: 5000,
  });
  const { data: status } = useQuery({
    queryKey: ["current_status"],
    queryFn: async () => (await invoker())<string>("current_status"),
    refetchInterval: 5000,
  });
  const { data: onBreak } = useQuery({
    queryKey: ["is_on_break"],
    queryFn: async () => (await invoker())<boolean>("is_on_break"),
    refetchInterval: 5000,
  });
  const { data: inMeeting } = useQuery({
    queryKey: ["is_in_meeting"],
    queryFn: async () => (await invoker())<boolean>("is_in_meeting"),
    refetchInterval: 5000,
  });

  async function run(fn: (invoke: Awaited<ReturnType<typeof invoker>>) => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      const invoke = await invoker();
      await fn(invoke);
      invoke("heartbeat_now").catch(() => {});
      await qc.invalidateQueries();
    } catch (e) {
      setError(typeof e === "string" ? e : "Action failed.");
    } finally {
      setBusy(false);
    }
  }

  const st = STATUS_LABEL[status ?? "not_working"] ?? { label: status ?? "—", dot: "bg-slate-400" };

  return (
    <section className="flex flex-col gap-4 rounded-lg border border-slate-200 p-6 dark:border-slate-800">
      <div className="flex items-center justify-between">
        <p className="text-sm text-slate-500">Tracking controls</p>
        <span className="inline-flex items-center gap-2 text-sm">
          <span className={`h-2.5 w-2.5 rounded-full ${st.dot}`} />
          {st.label}
        </span>
      </div>

      {onBreak ? (
        <button
          disabled={busy}
          onClick={() =>
            run(async (i) => {
              await i("set_break", { on: false });
              await i("start_tracking", { userId });
            })
          }
          className="rounded-md bg-blue-600 px-4 py-2 font-medium text-white hover:bg-blue-700 disabled:opacity-50"
        >
          Resume work
        </button>
      ) : !tracking ? (
        <button
          disabled={busy}
          onClick={() => run((i) => i("start_tracking", { userId }))}
          className="rounded-md bg-green-600 px-4 py-2 font-medium text-white hover:bg-green-700 disabled:opacity-50"
        >
          Start tracking
        </button>
      ) : (
        <div className="flex flex-col gap-3">
          <button
            disabled={busy}
            onClick={() =>
              run(async (i) => {
                await i("stop_tracking");
                await i("set_meeting", { on: false });
              })
            }
            className="rounded-md bg-red-600 px-4 py-2 font-medium text-white hover:bg-red-700 disabled:opacity-50"
          >
            Stop tracking
          </button>
          <div className="flex gap-3">
            <button
              disabled={busy}
              onClick={() =>
                run(async (i) => {
                  await i("stop_tracking");
                  await i("set_meeting", { on: false });
                  await i("set_break", { on: true });
                })
              }
              className="flex-1 rounded-md bg-slate-200 px-4 py-2 font-medium text-slate-800 hover:bg-slate-300 disabled:opacity-50 dark:bg-slate-800 dark:text-slate-100 dark:hover:bg-slate-700"
            >
              Break
            </button>
            <button
              disabled={busy}
              onClick={() => run((i) => i("set_meeting", { on: !inMeeting }))}
              className={`flex-1 rounded-md px-4 py-2 font-medium disabled:opacity-50 ${
                inMeeting
                  ? "bg-purple-600 text-white hover:bg-purple-700"
                  : "bg-slate-200 text-slate-800 hover:bg-slate-300 dark:bg-slate-800 dark:text-slate-100 dark:hover:bg-slate-700"
              }`}
            >
              {inMeeting ? "End meeting" : "Meeting mode"}
            </button>
          </div>
        </div>
      )}
      {error && <p className="text-sm text-red-600">{error}</p>}
    </section>
  );
}
