"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { fetchTeam } from "@/lib/api";
import { useAuthStore } from "@/lib/auth-store";
import { useAdminSession } from "@/components/use-admin-session";
import { STATUS_STYLES } from "@/lib/status";
import { fmtHms, timeAgo } from "@/lib/format";
import { cn } from "@/lib/utils";

export default function DashboardPage() {
  const router = useRouter();
  const { user, ready } = useAdminSession();
  const clear = useAuthStore((s) => s.clear);

  const team = useQuery({
    queryKey: ["team"],
    queryFn: () => fetchTeam(),
    refetchInterval: 30_000, // spec: refresh every 30s
    enabled: ready,
  });

  function signOut() {
    clear();
    router.replace("/login");
  }

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="container mx-auto flex max-w-4xl flex-col gap-6 py-12">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-3xl font-bold tracking-tight">TimeTracker Admin</h1>
          <p className="text-muted-foreground">
            Signed in as {user!.name} ·{" "}
            {user!.role === "hr" ? "HR (all employees)" : "Project manager (own team)"}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {user!.role === "hr" && (
            <Link
              href="/manage"
              className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90"
            >
              Manage users
            </Link>
          )}
          <button
            onClick={signOut}
            className="rounded-md bg-secondary px-4 py-2 text-sm font-medium hover:opacity-90"
          >
            Sign out
          </button>
        </div>
      </header>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold">Team — live status</h2>
          {team.dataUpdatedAt > 0 && (
            <span className="text-xs text-muted-foreground">
              updated {timeAgo(new Date(team.dataUpdatedAt).toISOString())} · auto-refresh 30s
            </span>
          )}
        </div>

        {team.isLoading && <p className="text-muted-foreground">Loading…</p>}
        {team.error && <p className="text-red-600">{(team.error as Error).message}</p>}
        {team.data && team.data.length === 0 && (
          <p className="text-muted-foreground">No team members.</p>
        )}

        {team.data && team.data.length > 0 && (
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b text-left text-muted-foreground">
                <th className="py-2 font-medium">Name</th>
                <th className="py-2 font-medium">Status</th>
                <th className="py-2 font-medium">Last seen</th>
                <th className="py-2 font-medium">Today&apos;s hours</th>
              </tr>
            </thead>
            <tbody>
              {team.data.map((m) => {
                const s = STATUS_STYLES[m.status];
                return (
                  <tr
                    key={m.user.id}
                    onClick={() => router.push(`/users/${m.user.id}`)}
                    className="cursor-pointer border-b last:border-0 hover:bg-muted/50"
                  >
                    <td className="py-2">
                      <div className="font-medium">{m.user.name}</div>
                      <div className="text-xs text-muted-foreground">{m.user.email}</div>
                    </td>
                    <td className="py-2">
                      <span className={cn("inline-flex items-center gap-2", s.text)}>
                        <span className={cn("h-2.5 w-2.5 rounded-full", s.dot)} />
                        {s.label}
                      </span>
                    </td>
                    <td className="py-2 text-muted-foreground">{timeAgo(m.last_seen_at)}</td>
                    <td className="py-2 tabular-nums">{fmtHms(m.today_seconds)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </section>
    </main>
  );
}
