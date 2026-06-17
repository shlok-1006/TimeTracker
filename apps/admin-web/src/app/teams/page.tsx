"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { fetchTeams } from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

export default function TeamsPage() {
  const { ready } = useAdminSession();
  const teams = useQuery({ queryKey: ["teams"], queryFn: fetchTeams, enabled: ready });

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="container mx-auto flex max-w-3xl flex-col gap-6 py-12">
      <header>
        <h1 className="text-2xl font-bold tracking-tight">Teams</h1>
      </header>

      {teams.isLoading && <p className="text-muted-foreground">Loading…</p>}
      {teams.error && <p className="text-red-600">{(teams.error as Error).message}</p>}
      {teams.data && teams.data.length === 0 && (
        <p className="text-muted-foreground">No teams yet. Create one via the team management API.</p>
      )}

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        {teams.data?.map((t) => (
          <Link
            key={t.id}
            href={`/teams/${t.id}`}
            className="rounded-lg border bg-card p-4 text-card-foreground transition hover:border-primary"
          >
            <div className="font-medium">{t.name}</div>
            {t.description && (
              <div className="text-sm text-muted-foreground">{t.description}</div>
            )}
            <div className="mt-2 text-xs text-muted-foreground">
              {t.member_count} member{t.member_count === 1 ? "" : "s"}
            </div>
          </Link>
        ))}
      </div>
    </main>
  );
}
