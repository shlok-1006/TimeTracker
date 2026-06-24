"use client";

import { useState } from "react";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  analyzeUserDay,
  analyzeUserRange,
  fetchUserHours,
  fetchUserReport,
  fetchUserDayScreenshots,
  fetchUserTeams,
  fetchUserTimeline,
} from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";
import { DayGallery } from "@/components/day-gallery";
import { ReportCard } from "@/components/report-card";
import { UserTasks } from "@/components/user-tasks";
import { ActivityTimeline } from "@/components/activity-timeline";
import { fmtHms } from "@/lib/format";

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border p-3">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="text-lg font-semibold tabular-nums">{value}</p>
    </div>
  );
}

export default function UserDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { ready } = useAdminSession();

  // Selected day (local). The timeline window is [local midnight, +24h).
  const [date, setDate] = useState(() => new Date().toLocaleDateString("en-CA"));
  const dayStart = new Date(`${date}T00:00:00`);
  const fromIso = dayStart.toISOString();
  const toIso = new Date(dayStart.getTime() + 86_400_000).toISOString();

  const timeline = useQuery({
    queryKey: ["user_timeline", id, date],
    queryFn: () => fetchUserTimeline(id, fromIso, toIso),
    enabled: ready && !!id,
    refetchInterval: 30_000,
  });

  const hours = useQuery({
    queryKey: ["user_hours", id],
    queryFn: () => fetchUserHours(id),
    enabled: ready && !!id,
  });
  const report = useQuery({
    queryKey: ["user_report", id, date],
    queryFn: () => fetchUserReport(id, date),
    enabled: ready && !!id,
  });
  const qc = useQueryClient();
  const analyze = useMutation({
    mutationFn: () => analyzeUserDay(id, date),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["user_report", id, date] });
      qc.invalidateQueries({ queryKey: ["user_day_screenshots", id, date] });
    },
  });
  // Deep analysis over a date + wall-clock time window (analyzes every working
  // screenshot in the window, not just the daily sample).
  const [range, setRange] = useState(() => {
    const today = new Date().toLocaleDateString("en-CA");
    return { from: today, to: today, start_time: "15:00", end_time: "19:00" };
  });
  const analyzeRange = useMutation({
    mutationFn: () => analyzeUserRange(id, range),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["user_report", id] });
      qc.invalidateQueries({ queryKey: ["user_day_screenshots", id] });
    },
  });
  const teams = useQuery({
    queryKey: ["user_teams", id],
    queryFn: () => fetchUserTeams(id),
    enabled: ready && !!id,
  });
  const shots = useQuery({
    queryKey: ["user_day_screenshots", id, date],
    queryFn: () => fetchUserDayScreenshots(id, date),
    enabled: ready && !!id,
  });

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="container mx-auto flex max-w-3xl flex-col gap-6 py-12">
      <header className="flex items-center justify-between">
        <h1 className="text-2xl font-bold tracking-tight">Employee detail</h1>
        <Link
          href="/dashboard"
          className="rounded-md bg-secondary px-4 py-2 text-sm font-medium hover:opacity-90"
        >
          ← Back to team
        </Link>
      </header>

      {(hours.error || shots.error) && (
        <p className="text-red-600">
          {((hours.error || shots.error) as Error).message}
          {" — you may not have access to this employee."}
        </p>
      )}

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold">Activity timeline</h2>
          <input
            type="date"
            value={date}
            onChange={(e) => setDate(e.target.value)}
            className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
          />
        </div>
        {timeline.isLoading && <p className="text-muted-foreground">Loading…</p>}
        {timeline.error && <p className="text-red-600">{(timeline.error as Error).message}</p>}
        {timeline.data && <ActivityTimeline data={timeline.data} />}
      </section>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-3 text-lg font-semibold">Hours</h2>
        {hours.isLoading && <p className="text-muted-foreground">Loading…</p>}
        {hours.data && (
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <Stat label="Today" value={fmtHms(hours.data.today_seconds)} />
            <Stat label="This week" value={fmtHms(hours.data.week_seconds)} />
            <Stat label="Active" value={fmtHms(hours.data.active_seconds)} />
            <Stat label="Idle" value={fmtHms(hours.data.idle_seconds)} />
          </div>
        )}
      </section>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 className="text-lg font-semibold">Daily report</h2>
          <div className="flex items-center gap-3">
            <span className="text-xs text-muted-foreground">{date}</span>
            <button
              onClick={() => analyze.mutate()}
              disabled={analyze.isPending}
              className="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
              title="Sample this day's working screenshots and analyze them with the AI"
            >
              {analyze.isPending ? "Analyzing…" : "Analyze now"}
            </button>
          </div>
        </div>
        {analyze.isError && (
          <p className="mb-3 text-sm text-red-600">
            {(analyze.error as Error).message}
          </p>
        )}
        {analyze.isSuccess && (
          <p className="mb-3 text-sm text-green-600">
            Analyzed {analyze.data.analyzed} screenshot
            {analyze.data.analyzed === 1 ? "" : "s"}
            {analyze.data.skipped > 0 ? ` (skipped ${analyze.data.skipped} meeting)` : ""}.
          </p>
        )}
        {/* Teams the employee belongs to (self-selected). */}
        <div className="mb-4 flex flex-wrap items-center gap-2">
          <span className="text-xs text-muted-foreground">Teams:</span>
          {teams.data && teams.data.length > 0 ? (
            teams.data.map((t) => (
              <Link
                key={t.id}
                href={`/teams/${t.id}`}
                className="rounded-full bg-secondary px-2.5 py-0.5 text-xs font-medium hover:opacity-80"
              >
                {t.name}
              </Link>
            ))
          ) : (
            <span className="text-xs text-muted-foreground">none</span>
          )}
        </div>
        {report.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
        {report.error && <p className="text-red-600">{(report.error as Error).message}</p>}
        {report.data !== undefined && <ReportCard report={report.data} />}
      </section>

      {/* Deep analysis: analyze every working screenshot in a time window. */}
      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-1 text-lg font-semibold">Deep analysis (time range)</h2>
        <p className="mb-4 text-xs text-muted-foreground">
          Analyzes <strong>every</strong> working screenshot taken between the chosen times,
          across the date range (not just the daily sample). Times are in your local timezone.
        </p>
        <div className="flex flex-wrap items-end gap-3">
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">From date</span>
            <input
              type="date"
              value={range.from}
              onChange={(e) => setRange({ ...range, from: e.target.value })}
              className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">To date</span>
            <input
              type="date"
              value={range.to}
              onChange={(e) => setRange({ ...range, to: e.target.value })}
              className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">From time</span>
            <input
              type="time"
              value={range.start_time}
              onChange={(e) => setRange({ ...range, start_time: e.target.value })}
              className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">To time</span>
            <input
              type="time"
              value={range.end_time}
              onChange={(e) => setRange({ ...range, end_time: e.target.value })}
              className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
            />
          </label>
          <button
            onClick={() => analyzeRange.mutate()}
            disabled={analyzeRange.isPending}
            className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
          >
            {analyzeRange.isPending ? "Analyzing…" : "Analyze range"}
          </button>
        </div>
        {analyzeRange.isError && (
          <p className="mt-3 text-sm text-red-600">{(analyzeRange.error as Error).message}</p>
        )}
        {analyzeRange.isSuccess && (
          <p className="mt-3 text-sm text-green-600">
            Analyzed {analyzeRange.data.analyzed} screenshot
            {analyzeRange.data.analyzed === 1 ? "" : "s"} across {analyzeRange.data.days} day
            {analyzeRange.data.days === 1 ? "" : "s"}
            {analyzeRange.data.skipped > 0 ? ` (skipped ${analyzeRange.data.skipped})` : ""}. Open
            a day in that range to see its updated report.
          </p>
        )}
      </section>

      <UserTasks userId={id} />

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-3 text-lg font-semibold">Screenshots</h2>
        {shots.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
        {shots.error && <p className="text-red-600">{(shots.error as Error).message}</p>}
        {shots.data && <DayGallery shots={shots.data} />}
      </section>
    </main>
  );
}
