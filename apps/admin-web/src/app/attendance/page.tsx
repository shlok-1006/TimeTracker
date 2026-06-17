"use client";

import { useState } from "react";
import Link from "next/link";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { fetchAttendanceReport, rollupAttendance } from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";
import { fmtHms } from "@/lib/format";

function monthStart(): string {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), 1).toLocaleDateString("en-CA");
}

export default function AttendancePage() {
  const { user, ready } = useAdminSession();
  const qc = useQueryClient();
  const isHr = user?.role === "hr";

  const [from, setFrom] = useState(monthStart);
  const [to, setTo] = useState(() => new Date().toLocaleDateString("en-CA"));

  const report = useQuery({
    queryKey: ["attendance_report", from, to],
    queryFn: () => fetchAttendanceReport(from, to),
    enabled: ready,
  });

  const rollup = useMutation({
    mutationFn: () => rollupAttendance(),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["attendance_report"] }),
  });

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  const rows = report.data?.employees ?? [];

  return (
    <main className="container mx-auto flex max-w-4xl flex-col gap-6 py-12">
      <header>
        <h1 className="text-2xl font-bold tracking-tight">Attendance</h1>
      </header>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <div className="mb-4 flex flex-wrap items-end justify-between gap-3">
          <div className="flex flex-wrap items-end gap-3">
            <label className="flex flex-col gap-1 text-xs">
              <span className="text-muted-foreground">From</span>
              <input
                type="date"
                value={from}
                onChange={(e) => setFrom(e.target.value)}
                className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
              />
            </label>
            <label className="flex flex-col gap-1 text-xs">
              <span className="text-muted-foreground">To</span>
              <input
                type="date"
                value={to}
                onChange={(e) => setTo(e.target.value)}
                className="rounded-md border border-input bg-background px-3 py-1.5 text-sm"
              />
            </label>
          </div>
          {isHr && (
            <button
              onClick={() => rollup.mutate()}
              disabled={rollup.isPending}
              className="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
              title="Recompute yesterday's attendance for every employee"
            >
              {rollup.isPending ? "Recomputing…" : "Recompute yesterday"}
            </button>
          )}
        </div>
        {rollup.isSuccess && (
          <p className="mb-3 text-sm text-green-600">
            Recomputed {rollup.data.employees} employee
            {rollup.data.employees === 1 ? "" : "s"} for {rollup.data.day}.
          </p>
        )}

        {report.isLoading && <p className="text-muted-foreground">Loading…</p>}
        {report.error && <p className="text-red-600">{(report.error as Error).message}</p>}
        {report.data && rows.length === 0 && (
          <p className="text-muted-foreground">No employees in scope.</p>
        )}

        {rows.length > 0 && (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b text-left text-muted-foreground">
                  <th className="py-2 font-medium">Employee</th>
                  <th className="py-2 text-center font-medium">Present</th>
                  <th className="py-2 text-center font-medium">Partial</th>
                  <th className="py-2 text-center font-medium">Absent</th>
                  <th className="py-2 text-center font-medium">Leave</th>
                  <th className="py-2 text-center font-medium">Holiday</th>
                  <th className="py-2 text-right font-medium">Worked</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.user_id} className="border-b last:border-0 hover:bg-muted/50">
                    <td className="py-2">
                      <Link href={`/users/${r.user_id}`} className="font-medium hover:underline">
                        {r.name}
                      </Link>
                      <div className="text-xs text-muted-foreground">{r.email}</div>
                    </td>
                    <td className="py-2 text-center tabular-nums text-green-700">{r.present}</td>
                    <td className="py-2 text-center tabular-nums text-amber-700">{r.partial}</td>
                    <td className="py-2 text-center tabular-nums text-red-700">{r.absent}</td>
                    <td className="py-2 text-center tabular-nums">{r.leave}</td>
                    <td className="py-2 text-center tabular-nums text-muted-foreground">
                      {r.holiday}
                    </td>
                    <td className="py-2 text-right tabular-nums">{fmtHms(r.worked_seconds)}</td>
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
