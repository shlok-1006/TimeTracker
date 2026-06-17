"use client";

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
import type { TeamSummary } from "@/lib/api";
import { fmtHms } from "@/lib/format";

const STATUS = [
  { key: "active", name: "Active", color: "#22c55e" },
  { key: "meeting", name: "Meeting", color: "#8b5cf6" },
  { key: "idle", name: "Idle", color: "#f59e0b" },
  { key: "break", name: "Break", color: "#3b82f6" },
] as const;

function Metric({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="rounded-lg border p-4">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="text-2xl font-semibold tabular-nums">{value}</p>
      {sub && <p className="text-xs text-muted-foreground">{sub}</p>}
    </div>
  );
}

/** Team summary: metric cards + status-breakdown donut + per-member bar chart. */
export function TeamSummaryView({ summary }: { summary: TeamSummary }) {
  const sb = summary.status_breakdown;
  const donut = STATUS.map((s) => ({
    name: s.name,
    value: sb[s.key as keyof typeof sb],
    color: s.color,
  })).filter((d) => d.value > 0);
  const bars = summary.members.map((m) => ({
    name: m.name,
    hours: +(m.worked_seconds / 3600).toFixed(2),
  }));
  const trackedTotal = sb.active + sb.idle + sb.meeting + sb.break;

  return (
    <div className="flex flex-col gap-6">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        <Metric label="Total hours" value={fmtHms(summary.total_seconds)} sub="active + meeting" />
        <Metric
          label="Active users"
          value={`${summary.active_users}/${summary.member_count}`}
          sub="contributed time"
        />
        <Metric label="Members" value={String(summary.member_count)} />
      </div>

      <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
        <div className="rounded-lg border p-4">
          <h3 className="mb-3 text-sm font-semibold">Status breakdown</h3>
          {trackedTotal === 0 ? (
            <p className="text-sm text-muted-foreground">No tracked time for this team yet.</p>
          ) : (
            <>
              <div className="h-56">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie
                      data={donut}
                      dataKey="value"
                      nameKey="name"
                      innerRadius={50}
                      outerRadius={78}
                      paddingAngle={2}
                      strokeWidth={0}
                    >
                      {donut.map((d) => (
                        <Cell key={d.name} fill={d.color} />
                      ))}
                    </Pie>
                    <Tooltip formatter={(v: number, n: string) => [fmtHms(v), n]} />
                  </PieChart>
                </ResponsiveContainer>
              </div>
              <div className="flex flex-wrap justify-center gap-x-4 gap-y-1 text-xs">
                {STATUS.map((s) => (
                  <span key={s.key} className="inline-flex items-center gap-1.5">
                    <span className="h-2 w-2 rounded-full" style={{ background: s.color }} />
                    {s.name} {fmtHms(sb[s.key as keyof typeof sb])}
                  </span>
                ))}
              </div>
            </>
          )}
        </div>

        <div className="rounded-lg border p-4">
          <h3 className="mb-3 text-sm font-semibold">Member totals (hours)</h3>
          {bars.length === 0 ? (
            <p className="text-sm text-muted-foreground">No members.</p>
          ) : (
            <div className="h-56">
              <ResponsiveContainer width="100%" height="100%">
                <BarChart data={bars} layout="vertical" margin={{ left: 8, right: 8 }}>
                  <XAxis type="number" fontSize={12} />
                  <YAxis type="category" dataKey="name" width={90} fontSize={11} />
                  <Tooltip formatter={(v: number) => [`${v} h`, "worked"]} />
                  <Bar dataKey="hours" fill="#9333ea" radius={[0, 4, 4, 0]} />
                </BarChart>
              </ResponsiveContainer>
            </div>
          )}
        </div>
      </div>

      <div className="rounded-lg border p-4">
        <h3 className="mb-3 text-sm font-semibold">Members</h3>
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b text-left text-muted-foreground">
              <th className="py-2 font-medium">Name</th>
              <th className="py-2 font-medium">Worked</th>
            </tr>
          </thead>
          <tbody>
            {summary.members.map((m) => (
              <tr key={m.user_id} className="border-b last:border-0">
                <td className="py-2">
                  <div className="font-medium">{m.name}</div>
                  <div className="text-xs text-muted-foreground">{m.email}</div>
                </td>
                <td className="py-2 tabular-nums">{fmtHms(m.worked_seconds)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
