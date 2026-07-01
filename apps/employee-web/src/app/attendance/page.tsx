"use client";

import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { fetchMyAttendance, type AttendanceDayRow } from "@/lib/api";
import { useEmployeeSession } from "@/components/use-employee-session";

const STATUS_STYLE: Record<string, { bg: string; label: string }> = {
  present: { bg: "bg-green-500 text-white", label: "Present" },
  partial: { bg: "bg-amber-400 text-slate-900", label: "Partial" },
  absent: { bg: "bg-red-500 text-white", label: "Absent" },
  leave: { bg: "bg-blue-500 text-white", label: "Leave" },
  holiday: { bg: "bg-purple-400 text-white", label: "Holiday" },
  weekend: { bg: "bg-slate-200 text-slate-500", label: "Weekend" },
};

const WEEKDAYS = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

/** First and last day-of-month strings (YYYY-MM-DD) for a given month offset. */
function monthRange(year: number, month: number) {
  const first = new Date(year, month, 1);
  const last = new Date(year, month + 1, 0);
  const fmt = (d: Date) => d.toLocaleDateString("en-CA");
  return { from: fmt(first), to: fmt(last), first, last };
}

export default function AttendancePage() {
  const { ready } = useEmployeeSession();
  const now = new Date();
  const [cursor, setCursor] = useState({ year: now.getFullYear(), month: now.getMonth() });
  const { from, to, first, last } = useMemo(() => monthRange(cursor.year, cursor.month), [cursor]);

  const cal = useQuery({
    queryKey: ["my_attendance", from, to],
    queryFn: () => fetchMyAttendance(from, to),
    enabled: ready,
  });

  const byDay = useMemo(() => {
    const m = new Map<string, AttendanceDayRow>();
    for (const d of cal.data?.days ?? []) m.set(d.day, d);
    return m;
  }, [cal.data]);

  const cells: (AttendanceDayRow | null)[] = [];
  for (let i = 0; i < first.getDay(); i++) cells.push(null);
  for (let day = 1; day <= last.getDate(); day++) {
    const key = new Date(cursor.year, cursor.month, day).toLocaleDateString("en-CA");
    cells.push(
      byDay.get(key) ?? {
        user_id: "",
        day: key,
        status: "",
        worked_seconds: 0,
        idle_seconds: 0,
        first_in_utc: null,
        last_out_utc: null,
        note: "",
      },
    );
  }

  const monthLabel = first.toLocaleDateString(undefined, { month: "long", year: "numeric" });
  const counts = (cal.data?.days ?? []).reduce<Record<string, number>>((acc, d) => {
    acc[d.status] = (acc[d.status] ?? 0) + 1;
    return acc;
  }, {});

  function shift(delta: number) {
    setCursor((c) => {
      const m = c.month + delta;
      return { year: c.year + Math.floor(m / 12), month: ((m % 12) + 12) % 12 };
    });
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
          <h1 className="text-2xl font-bold tracking-tight">Attendance</h1>
          <p className="text-muted-foreground">Your monthly attendance calendar.</p>
        </div>
        <div className="flex items-center gap-2 text-sm">
          <button onClick={() => shift(-1)} className="rounded-md px-2 py-1 hover:bg-secondary">
            ←
          </button>
          <span className="w-36 text-center font-medium">{monthLabel}</span>
          <button onClick={() => shift(1)} className="rounded-md px-2 py-1 hover:bg-secondary">
            →
          </button>
        </div>
      </header>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        {cal.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
        {cal.error && <p className="text-sm text-red-600">{(cal.error as Error).message}</p>}

        <div className="grid grid-cols-7 gap-1 text-center text-xs">
          {WEEKDAYS.map((w) => (
            <div key={w} className="py-1 font-medium text-muted-foreground">
              {w}
            </div>
          ))}
          {cells.map((c, i) =>
            c === null ? (
              <div key={`blank-${i}`} />
            ) : (
              <div
                key={c.day}
                title={`${c.day}${c.note ? ` · ${c.note}` : ""}`}
                className={`flex aspect-square flex-col items-center justify-center rounded-md text-xs ${
                  STATUS_STYLE[c.status]?.bg ?? "bg-muted text-muted-foreground"
                }`}
              >
                <span className="font-medium">{Number(c.day.slice(-2))}</span>
              </div>
            ),
          )}
        </div>

        <div className="mt-4 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
          {Object.entries(STATUS_STYLE).map(([k, v]) => (
            <span key={k} className="inline-flex items-center gap-1.5">
              <span className={`h-3 w-3 rounded-sm ${v.bg}`} />
              {v.label}
              {counts[k] ? ` (${counts[k]})` : ""}
            </span>
          ))}
        </div>
      </section>
    </main>
  );
}
