"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from "recharts";
import { invoker } from "@/lib/tauri";

type DayShot = {
  screenshot: { id: string; taken_at: string; captured_status: string };
  verdict: string | null;
  meeting_flag: boolean;
  presigned_url: string;
};

type Report = {
  total_analyzed: number;
  aligned_count: number;
  partially_count: number;
  not_aligned_count: number;
  inconclusive_count: number;
  alignment_score: number;
  summary_text: string;
  model: string;
} | null;

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

/** Employee day view: a day selector, the daily AI report summary, and the
 *  day's screenshots (each badged with its verdict or "Meeting · not analysed"). */
export function DayReport() {
  const [date, setDate] = useState(() => new Date().toLocaleDateString("en-CA"));
  const [zoom, setZoom] = useState<string | null>(null);

  const shots = useQuery({
    queryKey: ["me_day_shots", date],
    queryFn: async () => (await invoker())<DayShot[]>("me_screenshots", { day: date }),
  });
  const report = useQuery({
    queryKey: ["me_report", date],
    queryFn: async () => {
      const invoke = await invoker();
      const r = await invoke<{ report: Report }>("me_report", { day: date });
      return r.report;
    },
  });

  const rep = report.data;
  const donut =
    rep && rep.total_analyzed > 0
      ? VERDICTS.map((v) => ({ name: v.name, value: rep[v.key] as number, color: v.color })).filter(
          (d) => d.value > 0,
        )
      : [];

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-center justify-between">
        <h3 className="font-semibold">My day</h3>
        <input
          type="date"
          value={date}
          onChange={(e) => setDate(e.target.value)}
          className="rounded-md border border-slate-200 bg-white px-3 py-1.5 text-sm dark:border-slate-700 dark:bg-slate-900"
        />
      </div>

      {/* Daily report summary */}
      <div className="rounded-lg border border-slate-200 p-5 dark:border-slate-800">
        <h4 className="mb-3 font-medium">Daily report</h4>
        {report.isLoading && <p className="text-sm text-slate-500">Loading…</p>}
        {!report.isLoading && (!rep || rep.total_analyzed === 0) && (
          <p className="text-sm text-slate-500">No analysis for this day yet.</p>
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
                <span className="text-[10px] uppercase tracking-wide text-slate-400">aligned</span>
              </div>
            </div>
            <div className="flex flex-col gap-2">
              <p className="text-sm leading-relaxed">{rep.summary_text}</p>
              <p className="text-xs text-slate-400">
                {rep.total_analyzed} screenshot{rep.total_analyzed === 1 ? "" : "s"} analysed
                {rep.model ? ` · ${rep.model}` : ""}
              </p>
            </div>
          </div>
        )}
      </div>

      {/* Day screenshots */}
      <div className="rounded-lg border border-slate-200 p-5 dark:border-slate-800">
        <h4 className="mb-3 font-medium">Screenshots</h4>
        {shots.isLoading && <p className="text-sm text-slate-500">Loading…</p>}
        {shots.error && <p className="text-sm text-red-600">{(shots.error as Error).message}</p>}
        {shots.data && shots.data.length === 0 && (
          <p className="text-sm text-slate-500">No screenshots for this day.</p>
        )}
        {shots.data && shots.data.length > 0 && (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
            {shots.data.map((s) => (
              <div
                key={s.screenshot.id}
                className="overflow-hidden rounded-md border border-slate-200 dark:border-slate-700"
              >
                <button
                  onClick={() => setZoom(s.presigned_url)}
                  className="block w-full"
                  title={new Date(s.screenshot.taken_at).toLocaleString()}
                >
                  {/* eslint-disable-next-line @next/next/no-img-element */}
                  <img
                    src={s.presigned_url}
                    alt="screenshot"
                    className="h-28 w-full object-cover"
                    onError={(e) => ((e.currentTarget as HTMLImageElement).style.display = "none")}
                  />
                </button>
                <div className="flex items-center justify-between gap-1 px-2 py-1.5">
                  <span className="text-[10px] tabular-nums text-slate-400">
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
      </div>

      {zoom && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6"
          onClick={() => setZoom(null)}
        >
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={zoom} alt="screenshot" className="max-h-[80vh] w-auto" />
        </div>
      )}
    </div>
  );
}
