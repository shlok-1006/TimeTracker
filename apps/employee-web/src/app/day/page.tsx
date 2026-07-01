"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from "recharts";
import { fetchMyDayScreenshots, fetchMyReport } from "@/lib/api";
import { useEmployeeSession } from "@/components/use-employee-session";

const VERDICTS = [
  { key: "aligned_count", name: "Aligned", color: "#22c55e" },
  { key: "partially_count", name: "Partial", color: "#84cc16" },
  { key: "not_aligned_count", name: "Not aligned", color: "#ef4444" },
  { key: "inconclusive_count", name: "Inconclusive", color: "#94a3b8" },
] as const;

const VERDICT_BADGE: Record<string, string> = {
  aligned: "bg-green-100 text-green-800",
  partially_aligned: "bg-lime-100 text-lime-800",
  not_aligned: "bg-red-100 text-red-800",
  inconclusive: "bg-slate-100 text-slate-700",
};

function scoreColor(score: number) {
  if (score >= 75) return "text-green-600";
  if (score >= 50) return "text-amber-600";
  return "text-red-600";
}

export default function DayPage() {
  const { ready } = useEmployeeSession();
  const [date, setDate] = useState(() => new Date().toLocaleDateString("en-CA"));
  const [zoom, setZoom] = useState<string | null>(null);

  const report = useQuery({
    queryKey: ["my_report", date],
    queryFn: () => fetchMyReport(date),
    enabled: ready,
  });
  const shots = useQuery({
    queryKey: ["my_day_shots", date],
    queryFn: () => fetchMyDayScreenshots(date),
    enabled: ready,
  });

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  const rep = report.data;
  const donut =
    rep && rep.total_analyzed > 0
      ? VERDICTS.map((v) => ({ name: v.name, value: rep[v.key] as number, color: v.color })).filter(
          (d) => d.value > 0,
        )
      : [];

  return (
    <main className="container mx-auto flex max-w-4xl flex-col gap-6 py-12">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">My Day</h1>
          <p className="text-muted-foreground">Your daily alignment report and screenshots.</p>
        </div>
        <input
          type="date"
          value={date}
          onChange={(e) => setDate(e.target.value)}
          className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
        />
      </header>

      {/* Daily report summary */}
      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-3 text-lg font-semibold">Daily report</h2>
        {report.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
        {report.error && <p className="text-sm text-red-600">{(report.error as Error).message}</p>}
        {!report.isLoading && (!rep || rep.total_analyzed === 0) && (
          <p className="text-sm text-muted-foreground">No analysis for this day yet.</p>
        )}
        {rep && rep.total_analyzed > 0 && (
          <div className="grid gap-5 sm:grid-cols-[160px_1fr] sm:items-center">
            <div className="relative flex flex-col items-center">
              <div className="h-36 w-36">
                <ResponsiveContainer width="100%" height="100%">
                  <PieChart>
                    <Pie
                      data={donut}
                      dataKey="value"
                      nameKey="name"
                      innerRadius={44}
                      outerRadius={64}
                      paddingAngle={2}
                      strokeWidth={0}
                    >
                      {donut.map((d) => (
                        <Cell key={d.name} fill={d.color} />
                      ))}
                    </Pie>
                    <Tooltip />
                  </PieChart>
                </ResponsiveContainer>
              </div>
              <div className="pointer-events-none absolute top-[42px] flex h-16 w-36 flex-col items-center justify-center">
                <span className={`text-2xl font-bold tabular-nums ${scoreColor(rep.alignment_score)}`}>
                  {Math.round(rep.alignment_score)}%
                </span>
                <span className="text-[10px] uppercase tracking-wide text-muted-foreground">
                  aligned
                </span>
              </div>
            </div>
            <div className="flex flex-col gap-2">
              <p className="text-sm leading-relaxed">{rep.summary_text}</p>
              <p className="text-xs text-muted-foreground">
                {rep.total_analyzed} screenshot{rep.total_analyzed === 1 ? "" : "s"} analysed
                {rep.model ? ` · ${rep.model}` : ""}
              </p>
            </div>
          </div>
        )}
      </section>

      {/* Day screenshots */}
      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-3 text-lg font-semibold">Screenshots</h2>
        {shots.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
        {shots.error && <p className="text-sm text-red-600">{(shots.error as Error).message}</p>}
        {shots.data && shots.data.length === 0 && (
          <p className="text-sm text-muted-foreground">No screenshots for this day.</p>
        )}
        {shots.data && shots.data.length > 0 && (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            {shots.data.map((s) => (
              <div key={s.screenshot.id} className="overflow-hidden rounded-md border">
                <button
                  onClick={() => setZoom(s.presigned_url)}
                  className="block w-full"
                  title={new Date(s.screenshot.taken_at).toLocaleString()}
                >
                  {/* eslint-disable-next-line @next/next/no-img-element */}
                  <img
                    src={s.presigned_url}
                    alt="screenshot"
                    loading="lazy"
                    className="h-28 w-full object-cover"
                    onError={(e) => ((e.currentTarget as HTMLImageElement).style.display = "none")}
                  />
                </button>
                <div className="flex items-center justify-between gap-1 px-2 py-1.5">
                  <span className="text-[10px] tabular-nums text-muted-foreground">
                    {new Date(s.screenshot.taken_at).toLocaleTimeString([], {
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </span>
                  {s.meeting_flag ? (
                    <span className="rounded-full bg-purple-100 px-2 py-0.5 text-[10px] font-medium text-purple-800">
                      Meeting
                    </span>
                  ) : s.verdict ? (
                    <span
                      className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${
                        VERDICT_BADGE[s.verdict] ?? "bg-slate-100 text-slate-700"
                      }`}
                    >
                      {s.verdict.replace(/_/g, " ")}
                    </span>
                  ) : (
                    <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[10px] text-slate-500">
                      —
                    </span>
                  )}
                </div>
              </div>
            ))}
          </div>
        )}
      </section>

      {zoom && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6"
          onClick={() => setZoom(null)}
        >
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={zoom} alt="screenshot" className="max-h-[80vh] w-auto" />
        </div>
      )}
    </main>
  );
}
