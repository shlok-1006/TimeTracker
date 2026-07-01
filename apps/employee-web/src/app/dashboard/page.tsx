"use client";

import { useQuery } from "@tanstack/react-query";
import { Cell, Pie, PieChart, ResponsiveContainer, Tooltip } from "recharts";
import { fetchMyHours } from "@/lib/api";
import { useEmployeeSession } from "@/components/use-employee-session";
import { fmtHms } from "@/lib/format";

function Card({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border bg-card p-5 text-card-foreground">
      <p className="text-sm text-muted-foreground">{label}</p>
      <p className="mt-1 text-2xl font-semibold tabular-nums">{value}</p>
    </div>
  );
}

export default function DashboardPage() {
  const { ready } = useEmployeeSession();

  const hours = useQuery({
    queryKey: ["my_hours"],
    queryFn: () => fetchMyHours(),
    refetchInterval: 30_000,
    enabled: ready,
  });

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  const h = hours.data;
  const pieData = [
    { name: "Active", value: h?.active_seconds ?? 0, color: "#22c55e" },
    { name: "Idle", value: h?.idle_seconds ?? 0, color: "#f59e0b" },
  ];

  return (
    <main className="container mx-auto flex max-w-4xl flex-col gap-6 py-12">
      <header>
        <h1 className="text-2xl font-bold tracking-tight">Dashboard</h1>
        <p className="text-muted-foreground">Your work hours, at a glance.</p>
      </header>

      {hours.isLoading && <p className="text-muted-foreground">Loading…</p>}
      {hours.error && <p className="text-red-600">{(hours.error as Error).message}</p>}

      {h && (
        <>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            <Card label="Today's hours" value={fmtHms(h.today_seconds)} />
            <Card label="This week" value={fmtHms(h.week_seconds)} />
          </div>

          <section className="rounded-lg border bg-card p-6 text-card-foreground">
            <h2 className="mb-3 text-lg font-semibold">Active vs Idle (all time)</h2>
            <div className="h-56">
              <ResponsiveContainer width="100%" height="100%">
                <PieChart>
                  <Pie data={pieData} dataKey="value" nameKey="name" innerRadius={50} outerRadius={80}>
                    {pieData.map((d) => (
                      <Cell key={d.name} fill={d.color} />
                    ))}
                  </Pie>
                  <Tooltip formatter={(v: number) => fmtHms(v)} />
                </PieChart>
              </ResponsiveContainer>
            </div>
            <div className="flex justify-center gap-6 text-sm">
              <span className="inline-flex items-center gap-2">
                <span className="h-2.5 w-2.5 rounded-full bg-green-500" /> Active{" "}
                {fmtHms(h.active_seconds)}
              </span>
              <span className="inline-flex items-center gap-2">
                <span className="h-2.5 w-2.5 rounded-full bg-amber-500" /> Idle{" "}
                {fmtHms(h.idle_seconds)}
              </span>
            </div>
          </section>
        </>
      )}
    </main>
  );
}
