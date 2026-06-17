"use client";

import Link from "next/link";
import { useParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { fetchTeamSummary } from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";
import { TeamSummaryView } from "@/components/team-summary";

export default function TeamDetailPage() {
  const { id } = useParams<{ id: string }>();
  const { ready } = useAdminSession();

  const summary = useQuery({
    queryKey: ["team_summary", id],
    queryFn: () => fetchTeamSummary(id),
    enabled: ready && !!id,
    refetchInterval: 30_000,
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
        <div>
          <h1 className="text-2xl font-bold tracking-tight">
            {summary.data?.team.name ?? "Team summary"}
          </h1>
          {summary.data?.team.description && (
            <p className="text-sm text-muted-foreground">{summary.data.team.description}</p>
          )}
        </div>
        <Link
          href="/teams"
          className="rounded-md bg-secondary px-4 py-2 text-sm font-medium hover:opacity-90"
        >
          ← All teams
        </Link>
      </header>

      {summary.isLoading && <p className="text-muted-foreground">Loading…</p>}
      {summary.error && (
        <p className="text-red-600">
          {(summary.error as Error).message} — you may not have access to this team.
        </p>
      )}
      {summary.data && <TeamSummaryView summary={summary.data} />}
    </main>
  );
}
