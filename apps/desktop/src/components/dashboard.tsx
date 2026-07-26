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
  today_seconds: number;
  today_active_seconds: number;
  today_idle_seconds: number;
  today_meeting_seconds: number;
  week_seconds: number;
  week_active_seconds: number;
  week_idle_seconds: number;
  week_meeting_seconds: number;
  week_grace_seconds?: number;
  total_seconds: number;
};
type DayBucket = { date: string; worked_seconds: number; idle_seconds: number };
type ActivitySummary = {
  activity_pct: number | null;
  apps: { app_name: string; seconds: number }[];
};

function Card({
  label,
  value,
  hint,
}: {
  label: string;
  value: string;
  hint?: React.ReactNode;
}) {
  return (
    <div className="rounded-lg border border-slate-200 p-5 dark:border-slate-800">
      <p className="text-sm text-slate-500">{label}</p>
      <p className="mt-1 text-2xl font-semibold tabular-nums">{value}</p>
      {hint}
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
  const activity = useQuery({
    queryKey: ["activity_today"],
    queryFn: async () => (await invoker())<ActivitySummary>("activity_today"),
    refetchInterval: 30000,
  });

  const s = localSummary.data;
  const reconciled = serverHours.data?.total_seconds;
  // Grace time is server-side (local SQLite doesn't know about it); fold it into
  // the displayed week total and tag it.
  const weekGrace = serverHours.data?.week_grace_seconds ?? 0;
  const weekTotal = (s?.week_seconds ?? 0) + weekGrace;
  const statusInfo = STATUS_LABEL[status.data ?? "not_working"] ?? {
    label: status.data ?? "—",
    dot: "bg-slate-400",
  };

  const pieData = [
    { name: "Active", value: s?.today_active_seconds ?? 0, color: "#22c55e" },
    { name: "Idle", value: s?.today_idle_seconds ?? 0, color: "#f59e0b" },
    { name: "Meeting", value: s?.today_meeting_seconds ?? 0, color: "#6366f1" },
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
        <Card
          label="This week"
          value={fmtHms(weekTotal)}
          hint={
            weekGrace > 0 ? (
              <span
                className="mt-1 inline-block rounded bg-amber-100 px-1.5 py-0.5 text-[11px] font-medium text-amber-800"
                title="Includes grace time added by HR / your manager"
              >
                incl. {fmtHms(weekGrace)} grace
              </span>
            ) : undefined
          }
        />
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
          <h3 className="mb-3 font-semibold">Today: Active / Idle / Meeting</h3>
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
          <div className="flex flex-wrap justify-center gap-x-6 gap-y-1 text-sm">
            <span className="inline-flex items-center gap-2">
              <span className="h-2.5 w-2.5 rounded-full bg-green-500" /> Active{" "}
              {fmtHms(s?.today_active_seconds ?? 0)}
            </span>
            <span className="inline-flex items-center gap-2">
              <span className="h-2.5 w-2.5 rounded-full bg-amber-500" /> Idle{" "}
              {fmtHms(s?.today_idle_seconds ?? 0)}
            </span>
            <span className="inline-flex items-center gap-2">
              <span className="h-2.5 w-2.5 rounded-full bg-indigo-500" /> Meeting{" "}
              {fmtHms(s?.today_meeting_seconds ?? 0)}
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

      {/* Activity: what the app collects about you, shown to you (app names
          only — window titles and keystrokes are never recorded). */}
      <div className="rounded-lg border border-slate-200 p-5 dark:border-slate-800">
        <div className="mb-3 flex items-center justify-between">
          <h3 className="font-semibold">Today&apos;s activity</h3>
          <span className="text-2xl font-semibold tabular-nums">
            {activity.data?.activity_pct != null
              ? `${Math.round(activity.data.activity_pct)}%`
              : "—"}
          </span>
        </div>
        {activity.data && activity.data.apps.length > 0 ? (
          <div className="flex flex-col gap-2">
            {activity.data.apps.slice(0, 6).map((a) => {
              const max = activity.data!.apps[0].seconds || 1;
              return (
                <div key={a.app_name} className="flex items-center gap-3 text-sm">
                  <span className="w-40 truncate" title={a.app_name}>
                    {a.app_name}
                  </span>
                  <div className="h-2.5 flex-1 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
                    <div
                      className="h-full rounded-full bg-purple-600"
                      style={{ width: `${(a.seconds / max) * 100}%` }}
                    />
                  </div>
                  <span className="w-20 text-right tabular-nums text-slate-500">
                    {fmtHms(a.seconds)}
                  </span>
                </div>
              );
            })}
            <p className="mt-1 text-xs text-slate-400">
              App names only — window titles and keystrokes are never recorded.
            </p>
          </div>
        ) : (
          <p className="text-sm text-slate-500">
            No activity recorded yet today — data appears while you track.
          </p>
        )}
      </div>
    </div>
  );
}
