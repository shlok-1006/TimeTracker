"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { listAlumni } from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

const ROLE_LABEL: Record<string, string> = {
  employee: "Employee",
  project_manager: "Project manager",
  hr: "HR",
};

/** Format an ISO timestamp as a short local date (empty string if missing). */
function fmtDate(iso: string | null): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "—" : d.toLocaleDateString();
}

export default function AlumniPage() {
  const router = useRouter();
  const { user, ready } = useAdminSession();

  // HR-only page (the API enforces it too).
  useEffect(() => {
    if (ready && user && user.role !== "hr") router.replace("/dashboard");
  }, [ready, user, router]);

  const alumni = useQuery({
    queryKey: ["alumni"],
    queryFn: listAlumni,
    enabled: ready && user?.role === "hr",
  });

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="container mx-auto flex max-w-4xl flex-col gap-6 py-8 sm:py-12">
      <header>
        <h1 className="text-3xl font-bold tracking-tight">Alumni</h1>
        <p className="text-muted-foreground">
          Former employees, retained after removal. Most recently removed first.
        </p>
      </header>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        {alumni.isLoading && <p className="text-muted-foreground">Loading…</p>}
        {alumni.error && (
          <p className="text-red-600">{(alumni.error as Error).message}</p>
        )}
        {alumni.data && alumni.data.length === 0 && (
          <p className="text-muted-foreground">No former employees yet.</p>
        )}

        {alumni.data && alumni.data.length > 0 && (
          <div className="overflow-x-auto">
            <table className="w-full min-w-[36rem] text-sm">
              <thead>
                <tr className="border-b text-left text-muted-foreground">
                  <th className="py-2 font-medium">Name</th>
                  <th className="py-2 font-medium">Role</th>
                  <th className="py-2 font-medium">Joined</th>
                  <th className="py-2 font-medium">Removed</th>
                </tr>
              </thead>
              <tbody>
                {alumni.data.map((a) => (
                  <tr key={a.id} className="border-b last:border-0">
                    <td className="py-2">
                      <div className="font-medium">{a.name}</div>
                      <div className="text-xs text-muted-foreground">{a.email}</div>
                    </td>
                    <td className="py-2">{ROLE_LABEL[a.role] ?? a.role}</td>
                    <td className="py-2 text-muted-foreground">{fmtDate(a.joined_at)}</td>
                    <td className="py-2 text-muted-foreground">{fmtDate(a.removed_at)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
    </main>
  );
}
