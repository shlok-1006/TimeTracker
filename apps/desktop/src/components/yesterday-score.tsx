"use client";

import { useCallback, useEffect, useState } from "react";
import { invoker } from "@/lib/tauri";

/** Yesterday's AI work-verification report (same shape as `day-report`). */
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

/** localStorage key remembering which day's score the user has already seen, so
 *  the pop-up shows once per morning and not on every navigation. */
const SEEN_KEY = "tt.yesterdayScoreSeen";
/** The org day is IST (UTC+5:30) — the same key the nightly batch writes. We
 *  compute "yesterday" against that offset so it's correct regardless of the
 *  machine's own timezone. Mirror of the server's ORG_TZ_OFFSET_MINUTES. */
const ORG_OFFSET_MIN = 330;

function orgYesterday(): string {
  const ist = new Date(Date.now() + ORG_OFFSET_MIN * 60_000);
  ist.setUTCDate(ist.getUTCDate() - 1);
  return ist.toISOString().slice(0, 10);
}

function scoreColor(score: number) {
  if (score >= 75) return "text-green-600";
  if (score >= 50) return "text-amber-600";
  return "text-red-600";
}

function alreadySeen(day: string) {
  try {
    return localStorage.getItem(SEEN_KEY) === day;
  } catch {
    return false;
  }
}
function markSeen(day: string) {
  try {
    localStorage.setItem(SEEN_KEY, day);
  } catch {
    /* private mode / storage blocked — worst case it shows again, no harm */
  }
}

/**
 * Morning score pop-up. On the first dashboard view of a new day, shows the
 * employee their previous day's alignment score. Only appears when yesterday
 * actually has an analysed report (so it stays silent on weekends / days with no
 * work / before the 07:30 IST nightly analysis has run — it'll catch the next
 * time the window is focused). Dismissing marks the day seen so it won't nag.
 */
export function YesterdayScoreDialog() {
  const [rep, setRep] = useState<Report>(null);
  const [day, setDay] = useState("");
  const [open, setOpen] = useState(false);

  const check = useCallback(async () => {
    const y = orgYesterday();
    if (alreadySeen(y)) return;
    try {
      const invoke = await invoker();
      const r = await invoke<{ report: Report }>("me_report", { day: y });
      if (r?.report && r.report.total_analyzed > 0) {
        setRep(r.report);
        setDay(y);
        setOpen(true);
      }
    } catch {
      /* not in Tauri, offline, or not scored yet — retry on next focus */
    }
  }, []);

  useEffect(() => {
    check();
    // Also re-check when the window regains focus, so a machine left running
    // overnight still surfaces the new day's score when the user comes back.
    const onFocus = () => check();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [check]);

  function dismiss() {
    if (day) markSeen(day);
    setOpen(false);
  }

  if (!open || !rep) return null;

  const breakdown: [string, number, string][] = [
    ["Aligned", rep.aligned_count, "text-green-600"],
    ["Partial", rep.partially_count, "text-lime-600"],
    ["Not aligned", rep.not_aligned_count, "text-red-600"],
    ["Inconclusive", rep.inconclusive_count, "text-slate-500"],
  ];

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="dialog"
      aria-modal="true"
      aria-label="Yesterday's score"
      onClick={dismiss}
    >
      <div
        className="w-full max-w-md rounded-xl bg-white p-6 shadow-xl dark:bg-slate-900"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="text-lg font-semibold">Yesterday&apos;s score</h2>
        <p className="mt-0.5 text-xs text-slate-400">
          {new Date(`${day}T00:00:00`).toLocaleDateString(undefined, {
            weekday: "long",
            month: "short",
            day: "numeric",
          })}
        </p>

        <div className="mt-4 flex items-baseline gap-2">
          <span className={`text-5xl font-bold tabular-nums ${scoreColor(rep.alignment_score)}`}>
            {Math.round(rep.alignment_score)}%
          </span>
          <span className="text-sm text-slate-400">aligned</span>
        </div>

        {rep.summary_text && <p className="mt-3 text-sm leading-relaxed">{rep.summary_text}</p>}

        <div className="mt-4 grid grid-cols-4 gap-2 text-center">
          {breakdown.map(([label, count, color]) => (
            <div key={label} className="rounded-md border border-slate-200 py-2 dark:border-slate-700">
              <div className={`text-lg font-semibold tabular-nums ${color}`}>{count}</div>
              <div className="text-[10px] uppercase tracking-wide text-slate-400">{label}</div>
            </div>
          ))}
        </div>

        <p className="mt-3 text-xs text-slate-400">
          {rep.total_analyzed} screenshot{rep.total_analyzed === 1 ? "" : "s"} analysed
          {rep.model ? ` · ${rep.model}` : ""}
        </p>

        <button
          onClick={dismiss}
          className="mt-5 w-full rounded-md bg-purple-600 px-4 py-2 font-medium text-white hover:bg-purple-700"
        >
          Got it
        </button>
      </div>
    </div>
  );
}
