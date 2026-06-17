"use client";

import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

type AttendanceDay = {
  day: string; // YYYY-MM-DD
  status: string;
  worked_seconds: number;
  note: string;
};
type Calendar = { from: string; to: string; days: AttendanceDay[] };

const STATUS_STYLE: Record<string, { bg: string; label: string }> = {
  present: { bg: "bg-green-500 text-white", label: "Present" },
  partial: { bg: "bg-amber-400 text-slate-900", label: "Partial" },
  absent: { bg: "bg-red-500 text-white", label: "Absent" },
  leave: { bg: "bg-blue-500 text-white", label: "Leave" },
  holiday: { bg: "bg-purple-400 text-white", label: "Holiday" },
  weekend: { bg: "bg-slate-200 text-slate-500 dark:bg-slate-700 dark:text-slate-400", label: "Weekend" },
};

const WEEKDAYS = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];

/** First and last day-of-month strings (YYYY-MM-DD) for a given month offset. */
function monthRange(year: number, month: number) {
  const first = new Date(year, month, 1);
  const last = new Date(year, month + 1, 0);
  const fmt = (d: Date) => d.toLocaleDateString("en-CA");
  return { from: fmt(first), to: fmt(last), first, last };
}

/** Employee attendance calendar (Feature 6C): a month grid derived from the
 *  interval log, integrating leave and holidays. */
export function MyAttendance() {
  const now = new Date();
  const [cursor, setCursor] = useState({ year: now.getFullYear(), month: now.getMonth() });
  const { from, to, first, last } = useMemo(
    () => monthRange(cursor.year, cursor.month),
    [cursor],
  );

  const cal = useQuery({
    queryKey: ["me_attendance", from, to],
    queryFn: async () => (await invoker())<Calendar>("me_attendance", { from, to }),
  });

  const byDay = useMemo(() => {
    const m = new Map<string, AttendanceDay>();
    for (const d of cal.data?.days ?? []) m.set(d.day, d);
    return m;
  }, [cal.data]);

  // Build the grid: leading blanks for the first weekday, then each day.
  const cells: (AttendanceDay | null)[] = [];
  for (let i = 0; i < first.getDay(); i++) cells.push(null);
  for (let day = 1; day <= last.getDate(); day++) {
    const key = new Date(cursor.year, cursor.month, day).toLocaleDateString("en-CA");
    cells.push(byDay.get(key) ?? { day: key, status: "", worked_seconds: 0, note: "" });
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

  return (
    <section className="flex flex-col gap-4 rounded-lg border border-slate-200 p-6 dark:border-slate-800">
      <div className="flex items-center justify-between">
        <h2 className="font-semibold">Attendance</h2>
        <div className="flex items-center gap-2 text-sm">
          <button
            onClick={() => shift(-1)}
            className="rounded-md px-2 py-1 hover:bg-slate-100 dark:hover:bg-slate-800"
          >
            ←
          </button>
          <span className="w-36 text-center font-medium">{monthLabel}</span>
          <button
            onClick={() => shift(1)}
            className="rounded-md px-2 py-1 hover:bg-slate-100 dark:hover:bg-slate-800"
          >
            →
          </button>
        </div>
      </div>

      {cal.isLoading && <p className="text-sm text-slate-500">Loading…</p>}
      {cal.error && (
        <p className="text-sm text-red-600">
          {cal.error instanceof Error ? cal.error.message : String(cal.error)}
        </p>
      )}

      <div className="grid grid-cols-7 gap-1 text-center text-xs">
        {WEEKDAYS.map((w) => (
          <div key={w} className="py-1 font-medium text-slate-400">
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
                STATUS_STYLE[c.status]?.bg ?? "bg-slate-50 text-slate-400 dark:bg-slate-800/40"
              }`}
            >
              <span className="font-medium">{Number(c.day.slice(-2))}</span>
            </div>
          ),
        )}
      </div>

      {/* Legend + month summary */}
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
        {Object.entries(STATUS_STYLE).map(([k, v]) => (
          <span key={k} className="inline-flex items-center gap-1.5">
            <span className={`h-3 w-3 rounded-sm ${v.bg}`} />
            {v.label}
            {counts[k] ? ` (${counts[k]})` : ""}
          </span>
        ))}
      </div>
    </section>
  );
}
