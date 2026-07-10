"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { useMutation, useQuery } from "@tanstack/react-query";
import {
  fetchAnalysisRun,
  fetchTeam,
  previewAnalyzeRange,
  startAnalyzeRange,
} from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

/** Rough Claude Haiku cost per analyzed screenshot (image + prompt + verdict). */
const EST_COST_PER_SHOT_USD = 0.005;

type Mode = "day" | "range";

function todayLocal(): string {
  return new Date().toLocaleDateString("en-CA"); // YYYY-MM-DD
}

/** UTC [from, to) covering one calendar day — matches how the server keys
 *  screenshots and reports to days. */
function dayWindow(day: string): { fromIso: string; toIso: string } | null {
  const from = new Date(`${day}T00:00:00Z`);
  if (Number.isNaN(from.getTime())) return null;
  const to = new Date(from.getTime() + 24 * 60 * 60 * 1000);
  return { fromIso: from.toISOString(), toIso: to.toISOString() };
}

/** Local datetime-local inputs → UTC ISO window. */
function customWindow(from: string, to: string): { fromIso: string; toIso: string } | null {
  if (!from || !to) return null;
  const f = new Date(from);
  const t = new Date(to);
  if (Number.isNaN(f.getTime()) || Number.isNaN(t.getTime()) || t <= f) return null;
  return { fromIso: f.toISOString(), toIso: t.toISOString() };
}

export default function AnalyzePage() {
  const { ready } = useAdminSession();

  const [userId, setUserId] = useState("");
  const [mode, setMode] = useState<Mode>("day");
  const [day, setDay] = useState(todayLocal);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [runId, setRunId] = useState<string | null>(null);

  const team = useQuery({ queryKey: ["team"], queryFn: fetchTeam, enabled: ready });
  const employees = useMemo(
    () => (team.data ?? []).filter((m) => m.user.role === "employee"),
    [team.data],
  );

  const window_ = mode === "day" ? dayWindow(day) : customWindow(from, to);

  const preview = useQuery({
    queryKey: ["analyze_range_preview", userId, window_?.fromIso, window_?.toIso],
    queryFn: () => previewAnalyzeRange(userId, window_!.fromIso, window_!.toIso),
    enabled: ready && !!userId && !!window_,
  });

  const start = useMutation({
    mutationFn: () => startAnalyzeRange(userId, window_!.fromIso, window_!.toIso),
    onSuccess: (res) => setRunId(res.run_id),
  });

  const run = useQuery({
    queryKey: ["analysis_run", runId],
    queryFn: () => fetchAnalysisRun(runId!),
    enabled: !!runId,
    // Poll while running; stop once the run reaches a terminal state.
    refetchInterval: (q) => (q.state.data?.status === "running" ? 2000 : false),
  });

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  const p = preview.data;
  const overCap = !!p && p.analyzable > p.cap;
  const running = run.data?.status === "running";
  const canStart =
    !!userId && !!window_ && !!p && p.analyzable > 0 && !overCap && p.claude_configured &&
    !start.isPending && !running;

  const done = run.data ? run.data.analyzed + run.data.skipped + run.data.failed : 0;
  const pct = run.data && run.data.total > 0 ? Math.min(100, Math.round((done / run.data.total) * 100)) : 0;

  return (
    <main className="container mx-auto flex max-w-4xl flex-col gap-6 py-8 sm:py-12">
      <header>
        <h1 className="text-2xl font-bold tracking-tight">Analyze screenshots</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Verify every working screenshot in a day or time range against the
          employee&apos;s assigned tickets.
        </p>
      </header>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <div className="flex flex-wrap items-end gap-3">
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Employee</span>
            <select
              value={userId}
              onChange={(e) => { setUserId(e.target.value); setRunId(null); }}
              className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
            >
              <option value="">Select an employee…</option>
              {employees.map((m) => (
                <option key={m.user.id} value={m.user.id}>
                  {m.user.name} ({m.user.email})
                </option>
              ))}
            </select>
          </label>

          <div className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Range type</span>
            <div className="flex overflow-hidden rounded-md border border-input">
              {(["day", "range"] as const).map((m) => (
                <button
                  key={m}
                  onClick={() => { setMode(m); setRunId(null); }}
                  className={
                    mode === m
                      ? "bg-primary px-3 py-1.5 text-sm font-medium text-primary-foreground"
                      : "bg-background px-3 py-1.5 text-sm hover:bg-secondary"
                  }
                >
                  {m === "day" ? "Whole day" : "Time range"}
                </button>
              ))}
            </div>
          </div>

          {mode === "day" ? (
            <label className="flex flex-col gap-1 text-xs">
              <span className="text-muted-foreground">Day (UTC)</span>
              <input
                type="date"
                value={day}
                onChange={(e) => { setDay(e.target.value); setRunId(null); }}
                className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
              />
            </label>
          ) : (
            <>
              <label className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">From</span>
                <input
                  type="datetime-local"
                  value={from}
                  onChange={(e) => { setFrom(e.target.value); setRunId(null); }}
                  className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
                />
              </label>
              <label className="flex flex-col gap-1 text-xs">
                <span className="text-muted-foreground">To</span>
                <input
                  type="datetime-local"
                  value={to}
                  onChange={(e) => { setTo(e.target.value); setRunId(null); }}
                  className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
                />
              </label>
            </>
          )}
        </div>

        {mode === "range" && from && to && !window_ && (
          <p className="mt-3 text-sm text-red-600">&quot;To&quot; must be after &quot;From&quot;.</p>
        )}

        {/* Preview: counts + estimated cost, before any money is spent */}
        {userId && window_ && (
          <div className="mt-5 rounded-md border bg-secondary/40 p-4 text-sm">
            {preview.isLoading && <p className="text-muted-foreground">Counting screenshots…</p>}
            {preview.isError && (
              <p className="text-red-600">{(preview.error as Error).message}</p>
            )}
            {p && (
              <div className="flex flex-col gap-1">
                <p>
                  <span className="font-medium">{p.total}</span> screenshot{p.total === 1 ? "" : "s"} in
                  this window, <span className="font-medium">{p.analyzable}</span> analyzable
                  (working){p.total > p.analyzable ? " — meeting/break/idle shots are skipped" : ""}.
                </p>
                {p.analyzable > 0 && (
                  <p className="text-muted-foreground">
                    Estimated cost ≈ ${(p.analyzable * EST_COST_PER_SHOT_USD).toFixed(2)} ({p.model})
                  </p>
                )}
                {overCap && (
                  <p className="text-red-600">
                    Above the per-run cap of {p.cap} — narrow the range.
                  </p>
                )}
                {!p.claude_configured && (
                  <p className="text-amber-600">
                    Vision AI is not configured on the server (ANTHROPIC_API_KEY) — analysis
                    cannot run yet.
                  </p>
                )}
              </div>
            )}
          </div>
        )}

        <div className="mt-5">
          <button
            onClick={() => start.mutate()}
            disabled={!canStart}
            className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
          >
            {start.isPending
              ? "Starting…"
              : p && p.analyzable > 0
                ? `Analyze ${p.analyzable} screenshot${p.analyzable === 1 ? "" : "s"}`
                : "Analyze"}
          </button>
          {start.isError && (
            <p className="mt-2 text-sm text-red-600">{(start.error as Error).message}</p>
          )}
        </div>
      </section>

      {/* Live progress of the launched run */}
      {runId && (
        <section className="rounded-lg border bg-card p-6 text-card-foreground">
          <h2 className="mb-3 text-sm font-semibold">Run progress</h2>
          {run.isLoading && <p className="text-sm text-muted-foreground">Loading run…</p>}
          {run.data && (
            <div className="flex flex-col gap-3">
              <div className="h-3 w-full overflow-hidden rounded-full bg-secondary">
                <div
                  className={
                    run.data.status === "failed"
                      ? "h-full bg-red-500 transition-all"
                      : "h-full bg-primary transition-all"
                  }
                  style={{ width: `${pct}%` }}
                />
              </div>
              <p className="text-sm">
                {done} / {run.data.total} processed — {run.data.analyzed} analyzed,{" "}
                {run.data.skipped} skipped, {run.data.failed} failed
                {run.data.status === "running" && "…"}
              </p>
              {run.data.status === "completed" && (
                <p className="text-sm text-green-600">
                  Done.{" "}
                  <Link href={`/users/${run.data.user_id}`} className="underline">
                    View the verdicts on the employee&apos;s page →
                  </Link>
                </p>
              )}
              {run.data.status === "failed" && (
                <p className="text-sm text-red-600">
                  Run failed{run.data.error ? `: ${run.data.error}` : "."}
                </p>
              )}
            </div>
          )}
        </section>
      )}
    </main>
  );
}
