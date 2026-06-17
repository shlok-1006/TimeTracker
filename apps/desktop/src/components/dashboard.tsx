"use client";

import { useQuery } from "@tanstack/react-query";
import {
  Bar,
  BarChart,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { invoker, fmtHms, STATUS_LABEL } from "@/lib/tauri";

type HoursSummary = {
  total_seconds: number;
  today_seconds: number;
  week_seconds: number;
  active_seconds: number;
  idle_seconds: number;
};
type DayBucket = { date: string; worked_seconds: number; idle_seconds: number };

function Card({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-slate-200 p-5 dark:border-slate-800">
      <p className="text-sm text-slate-500">{label}</p>
      <p className="mt-1 text-2xl font-semibold tabular-nums">{value}</p>
    </div>
  );
}

/** Employee dashboard: cards + charts (local-first). The day-based screenshot
 *  gallery + daily report live in the `DayReport` component below. */
export function Dashboard({ userId }: { userId: string }) {
  const localSummary = useQuery({
    queryKey: ["hours_summary", userId],
    queryFn: async () => (await invoker())<HoursSummary>("get_hours_summary", { userId }),
    refetchInterval: 15000,
  });
  const timeline = useQuery({
    queryKey: ["daily_timeline", userId],
    queryFn: async () => (await invoker())<DayBucket[]>("get_daily_timeline", { userId }),
    refetchInterval: 60000,
  });
  const status = useQuery({
    queryKey: ["current_status"],
    queryFn: async () => (await invoker())<string>("current_status"),
    refetchInterval: 5000,
  });
  const serverHours = useQuery({
    queryKey: ["me_hours"],
    queryFn: async () => (await invoker())<HoursSummary>("me_hours"),
    refetchInterval: 30000,
  });

  const s = localSummary.data;
  const reconciled = serverHours.data?.total_seconds;
  const statusInfo = STATUS_LABEL[status.data ?? "not_working"] ?? {
    label: status.data ?? "—",
    dot: "bg-slate-400",
  };

  const pieData = [
    { name: "Active", value: s?.active_seconds ?? 0, color: "#22c55e" },
    { name: "Idle", value: s?.idle_seconds ?? 0, color: "#f59e0b" },
  ];
  const barData =
    timeline.data?.map((d) => ({
      day: d.date.slice(5),
      hours: +(d.worked_seconds / 3600).toFixed(2),
    })) ?? [];

  return (
    <div className="flex flex-col gap-6">
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <Card label="Today's hours" value={fmtHms(s?.today_seconds ?? 0)} />
        <Card label="This week" value={fmtHms(s?.week_seconds ?? 0)} />
        <div className="rounded-lg border border-slate-200 p-5 dark:border-slate-800">
          <p className="text-sm text-slate-500">Current status</p>
          <p className="mt-1 inline-flex items-center gap-2 text-2xl font-semibold">
            <span className={`h-3 w-3 rounded-full ${statusInfo.dot}`} />
            {statusInfo.label}
          </p>
        </div>
      </div>

      <p className="text-xs text-slate-400">
        Showing local data.{" "}
        {reconciled !== undefined
          ? `Server total: ${fmtHms(reconciled)} (reconciled).`
          : "Reconciling with server…"}
      </p>

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <div className="rounded-lg border border-slate-200 p-5 dark:border-slate-800">
          <h3 className="mb-3 font-semibold">Active vs Idle</h3>
          <div className="h-56">
            <ResponsiveContainer width="100%" height="100%">
              <PieChart>
                <Pie data={pieData} dataKey="value" nameKey="name" innerRadius={50} outerRadius={80}>
                  {pieData.map((d) => (
                    <Cell key={d.name} fill={d.color} />
                  ))}
                </Pie>
                <Tooltip formatter={(v: number) => fmtHms(v)} />
              </PieChart>
            </ResponsiveContainer>
          </div>
          <div className="flex justify-center gap-6 text-sm">
            <span className="inline-flex items-center gap-2">
              <span className="h-2.5 w-2.5 rounded-full bg-green-500" /> Active{" "}
              {fmtHms(s?.active_seconds ?? 0)}
            </span>
            <span className="inline-flex items-center gap-2">
              <span className="h-2.5 w-2.5 rounded-full bg-amber-500" /> Idle{" "}
              {fmtHms(s?.idle_seconds ?? 0)}
            </span>
          </div>
        </div>

        <div className="rounded-lg border border-slate-200 p-5 dark:border-slate-800">
          <h3 className="mb-3 font-semibold">Daily timeline (hours)</h3>
          <div className="h-56">
            <ResponsiveContainer width="100%" height="100%">
              <BarChart data={barData}>
                <XAxis dataKey="day" fontSize={12} />
                <YAxis fontSize={12} />
                <Tooltip />
                <Bar dataKey="hours" fill="#9333ea" radius={[4, 4, 0, 0]} />
              </BarChart>
            </ResponsiveContainer>
          </div>
        </div>
      </div>
    </div>
  );
}
