"use client";

import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from "recharts";
import type { DailyReport } from "@/lib/api";

const VERDICTS = [
  { key: "aligned_count", name: "Aligned", color: "#22c55e" },
  { key: "partially_count", name: "Partial", color: "#84cc16" },
  { key: "not_aligned_count", name: "Not aligned", color: "#ef4444" },
  { key: "inconclusive_count", name: "Inconclusive", color: "#94a3b8" },
] as const;

function scoreColor(score: number) {
  if (score >= 75) return "text-green-600";
  if (score >= 50) return "text-amber-600";
  return "text-red-600";
}

/** HR daily report card: alignment score, verdict donut, AI summary, counts. */
export function ReportCard({ report }: { report: DailyReport | null }) {
  if (!report || report.total_analyzed === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No analysis for this day yet — run the analyzer or pick another day.
      </p>
    );
  }

  const data = VERDICTS.map((v) => ({
    name: v.name,
    value: report[v.key] as number,
    color: v.color,
  })).filter((d) => d.value > 0);

  return (
    <div className="grid gap-6 sm:grid-cols-[200px_1fr] sm:items-center">
      {/* Donut + score */}
      <div className="relative flex flex-col items-center">
        <div className="h-44 w-44">
          <ResponsiveContainer width="100%" height="100%">
            <PieChart>
              <Pie
                data={data}
                dataKey="value"
                nameKey="name"
                innerRadius={54}
                outerRadius={76}
                paddingAngle={2}
                strokeWidth={0}
              >
                {data.map((d) => (
                  <Cell key={d.name} fill={d.color} />
                ))}
              </Pie>
              <Tooltip formatter={(v: number, n: string) => [`${v}`, n]} />
            </PieChart>
          </ResponsiveContainer>
        </div>
        {/* Centered score overlay */}
        <div className="pointer-events-none absolute top-[52px] flex h-20 w-44 flex-col items-center justify-center">
          <span className={`text-3xl font-bold tabular-nums ${scoreColor(report.alignment_score)}`}>
            {Math.round(report.alignment_score)}%
          </span>
          <span className="text-[10px] uppercase tracking-wide text-muted-foreground">aligned</span>
        </div>
      </div>

      {/* Summary + legend */}
      <div className="flex flex-col gap-3">
        <p className="text-sm leading-relaxed">{report.summary_text}</p>
        <div className="flex flex-wrap gap-2">
          {VERDICTS.map((v) => (
            <span
              key={v.key}
              className="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs"
            >
              <span className="h-2 w-2 rounded-full" style={{ background: v.color }} />
              {v.name}: <b className="tabular-nums">{report[v.key] as number}</b>
            </span>
          ))}
        </div>
        <p className="text-xs text-muted-foreground">
          {report.total_analyzed} screenshot{report.total_analyzed === 1 ? "" : "s"} analysed
          {report.model ? ` · ${report.model}` : ""}
        </p>
      </div>
    </div>
  );
}
