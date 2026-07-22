"use client";

import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ATTENDANCE_STATUSES,
  clearUserAttendance,
  fetchUserAttendance,
  setUserAttendance,
  type AttendanceDayRow,
  type AttendanceStatus,
} from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

const STATUS_LABEL: Record<string, string> = {
  present: "Present",
  partial: "Partial (half day)",
  absent: "Absent",
  leave: "Leave",
  holiday: "Holiday",
  weekend: "Weekend",
};

const pad = (n: number) => String(n).padStart(2, "0");

/** HR-editable attendance list for one employee, one month at a time.
 *  Editing pins a manual override that the nightly rollup won't overwrite;
 *  "Revert" restores the auto-derived status. Non-HR admins see it read-only. */
export function UserAttendance({ userId }: { userId: string }) {
  const qc = useQueryClient();
  const { user } = useAdminSession();
  const isHr = user?.role === "hr";

  const [month, setMonth] = useState(() => {
    const d = new Date();
    return { y: d.getFullYear(), m: d.getMonth() }; // m: 0-11
  });

  const { from, to, label } = useMemo(() => {
    const first = `${month.y}-${pad(month.m + 1)}-01`;
    const lastDay = new Date(month.y, month.m + 1, 0).getDate();
    const last = `${month.y}-${pad(month.m + 1)}-${pad(lastDay)}`;
    const label = new Date(month.y, month.m, 1).toLocaleDateString(undefined, {
      month: "long",
      year: "numeric",
    });
    return { from: first, to: last, label };
  }, [month]);

  const cal = useQuery({
    queryKey: ["user_attendance", userId, from, to],
    queryFn: () => fetchUserAttendance(userId, from, to),
    enabled: !!userId,
  });

  const shiftMonth = (delta: number) =>
    setMonth(({ y, m }) => {
      const d = new Date(y, m + delta, 1);
      return { y: d.getFullYear(), m: d.getMonth() };
    });

  const invalidate = () =>
    qc.invalidateQueries({ queryKey: ["user_attendance", userId, from, to] });

  return (
    <section className="rounded-lg border bg-card p-6 text-card-foreground">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <h2 className="text-lg font-semibold">Attendance</h2>
        <div className="flex items-center gap-2 text-sm">
          <button
            onClick={() => shiftMonth(-1)}
            className="rounded-md border px-2 py-1 hover:bg-secondary"
            aria-label="Previous month"
          >
            ←
          </button>
          <span className="min-w-[9rem] text-center font-medium tabular-nums">{label}</span>
          <button
            onClick={() => shiftMonth(1)}
            className="rounded-md border px-2 py-1 hover:bg-secondary"
            aria-label="Next month"
          >
            →
          </button>
        </div>
      </div>

      {cal.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
      {cal.error && <p className="text-sm text-red-600">{(cal.error as Error).message}</p>}
      {cal.data && cal.data.days.length === 0 && (
        <p className="text-sm text-muted-foreground">No attendance recorded this month.</p>
      )}
      {cal.data && cal.data.days.length > 0 && (
        <div className="overflow-x-auto">
          <table className="w-full min-w-[36rem] text-sm">
            <thead>
              <tr className="border-b text-left text-muted-foreground">
                <th className="py-2 font-medium">Date</th>
                <th className="py-2 font-medium">Status</th>
                <th className="py-2 font-medium">Worked</th>
                <th className="py-2 font-medium">Note</th>
                {isHr && <th className="py-2 font-medium" />}
              </tr>
            </thead>
            <tbody>
              {cal.data.days.map((d) => (
                <AttendanceRow
                  key={d.day}
                  row={d}
                  isHr={isHr}
                  onSaved={invalidate}
                  userId={userId}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function AttendanceRow({
  row,
  isHr,
  userId,
  onSaved,
}: {
  row: AttendanceDayRow;
  isHr: boolean;
  userId: string;
  onSaved: () => void;
}) {
  const [status, setStatus] = useState<AttendanceStatus>(row.status as AttendanceStatus);
  const [note, setNote] = useState(row.note);

  const save = useMutation({
    mutationFn: () => setUserAttendance(userId, row.day, status, note.trim()),
    onSuccess: onSaved,
  });
  const revert = useMutation({
    mutationFn: () => clearUserAttendance(userId, row.day),
    onSuccess: onSaved,
  });
  const busy = save.isPending || revert.isPending;
  const dirty = status !== row.status || note.trim() !== row.note;

  const weekday = new Date(`${row.day}T00:00:00`).toLocaleDateString(undefined, {
    weekday: "short",
  });

  return (
    <tr className="border-b last:border-0 align-top">
      <td className="py-2 whitespace-nowrap">
        <span className="tabular-nums">{row.day}</span>{" "}
        <span className="text-xs text-muted-foreground">{weekday}</span>
        {row.is_override && (
          <span className="ml-2 rounded bg-amber-100 px-1.5 py-0.5 text-[10px] font-medium text-amber-800">
            edited
          </span>
        )}
      </td>
      <td className="py-2">
        {isHr ? (
          <select
            value={status}
            onChange={(e) => setStatus(e.target.value as AttendanceStatus)}
            disabled={busy}
            className="rounded-md border border-input bg-background px-2 py-1 text-sm disabled:opacity-50"
          >
            {ATTENDANCE_STATUSES.map((s) => (
              <option key={s} value={s}>
                {STATUS_LABEL[s]}
              </option>
            ))}
          </select>
        ) : (
          <span>{STATUS_LABEL[row.status] ?? row.status}</span>
        )}
      </td>
      <td className="py-2 tabular-nums text-muted-foreground">
        {(row.worked_seconds / 3600).toFixed(1)}h
      </td>
      <td className="py-2">
        {isHr ? (
          <input
            value={note}
            onChange={(e) => setNote(e.target.value)}
            disabled={busy}
            placeholder="—"
            className="w-full min-w-[8rem] rounded-md border border-input bg-background px-2 py-1 text-sm disabled:opacity-50"
          />
        ) : (
          <span className="text-muted-foreground">{row.note || "—"}</span>
        )}
      </td>
      {isHr && (
        <td className="py-2">
          <div className="flex items-center justify-end gap-2">
            <button
              onClick={() => save.mutate()}
              disabled={busy || !dirty}
              className="rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
            >
              Save
            </button>
            {row.is_override && (
              <button
                onClick={() => revert.mutate()}
                disabled={busy}
                className="text-xs underline hover:opacity-80 disabled:opacity-50"
              >
                Revert
              </button>
            )}
          </div>
          {(save.isError || revert.isError) && (
            <p className="mt-1 text-right text-[11px] text-red-600">
              {((save.error || revert.error) as Error).message}
            </p>
          )}
        </td>
      )}
    </tr>
  );
}
