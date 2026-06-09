"use client";

import { useState } from "react";
import Link from "next/link";
import { useParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { fetchUserHours, fetchUserScreenshots, fetchUserTimeline } from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";
import { ScreenshotGallery } from "@/components/screenshot-gallery";
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
  const shots = useQuery({
    queryKey: ["user_screenshots", id],
    queryFn: () => fetchUserScreenshots(id),
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
        <h2 className="mb-3 text-lg font-semibold">Screenshots</h2>
        {shots.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
        {shots.data && <ScreenshotGallery shots={shots.data} />}
      </section>
    </main>
  );
}
