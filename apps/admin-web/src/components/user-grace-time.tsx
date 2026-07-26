"use client";

import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { addTimeGrant, deleteTimeGrant, fetchTimeGrants } from "@/lib/api";

function fmtHm(seconds: number) {
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return [h ? `${h}h` : null, m ? `${m}m` : null].filter(Boolean).join(" ") || "0m";
}

/** HR/PM manual "grace" time for the employee's current week. Each grant needs a
 *  reason; the total is folded into the week's hours and tagged as grace. */
export function UserGraceTime({ userId }: { userId: string }) {
  const qc = useQueryClient();
  const grants = useQuery({
    queryKey: ["time_grants", userId],
    queryFn: () => fetchTimeGrants(userId),
  });

  const [hours, setHours] = useState(0);
  const [minutes, setMinutes] = useState(0);
  const [reason, setReason] = useState("");
  const [err, setErr] = useState<string | null>(null);

  // Refresh both this panel and the hours summary (its grace tag/total).
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["time_grants", userId] });
    qc.invalidateQueries({ queryKey: ["user_hours", userId] });
  };

  const add = useMutation({
    mutationFn: () => addTimeGrant(userId, { hours, minutes, reason: reason.trim() }),
    onSuccess: () => {
      setHours(0);
      setMinutes(0);
      setReason("");
      setErr(null);
      invalidate();
    },
    onError: (e) => setErr(e instanceof Error ? e.message : "Failed to add grace time."),
  });
  const remove = useMutation({
    mutationFn: (id: string) => deleteTimeGrant(id),
    onSuccess: invalidate,
  });

  function submit(e: FormEvent) {
    e.preventDefault();
    if ((hours > 0 || minutes > 0) && reason.trim()) add.mutate();
  }

  const total = (grants.data?.grants ?? []).reduce((a, g) => a + g.seconds, 0);
  const input = "rounded-md border border-input bg-background px-3 py-1.5 text-sm";

  return (
    <section className="rounded-lg border bg-card p-6 text-card-foreground">
      <div className="mb-1 flex items-center justify-between gap-2">
        <h2 className="text-lg font-semibold">Grace time (this week)</h2>
        {total > 0 && (
          <span className="rounded bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-800">
            +{fmtHm(total)} granted
          </span>
        )}
      </div>
      <p className="mb-4 text-xs text-muted-foreground">
        Manually add time to this week&apos;s total (e.g. approved off-tracker work). A reason is
        required; the total is tagged as grace.
      </p>

      <form onSubmit={submit} className="mb-4 flex flex-wrap items-end gap-3">
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Hours</span>
          <input
            type="number"
            min={0}
            max={168}
            value={hours}
            onChange={(e) => setHours(Math.max(0, Number(e.target.value) || 0))}
            className={`${input} w-20`}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Minutes</span>
          <input
            type="number"
            min={0}
            max={59}
            value={minutes}
            onChange={(e) => setMinutes(Math.max(0, Math.min(59, Number(e.target.value) || 0)))}
            className={`${input} w-20`}
          />
        </label>
        <label className="flex flex-1 flex-col gap-1 text-xs">
          <span className="text-muted-foreground">Reason</span>
          <input
            required
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            placeholder="Reason (required)"
            className={`${input} w-full min-w-[10rem]`}
          />
        </label>
        <button
          type="submit"
          disabled={add.isPending || (hours === 0 && minutes === 0) || !reason.trim()}
          className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
        >
          Add
        </button>
      </form>
      {err && <p className="mb-2 text-sm text-red-600">{err}</p>}

      {grants.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
      {grants.data && grants.data.grants.length === 0 && (
        <p className="text-sm text-muted-foreground">No grace time added this week.</p>
      )}
      <ul className="flex flex-col gap-2">
        {grants.data?.grants.map((g) => (
          <li
            key={g.id}
            className="flex items-start justify-between gap-3 rounded-md border p-3 text-sm"
          >
            <div>
              <p className="font-medium">
                +{fmtHm(g.seconds)}{" "}
                <span className="font-normal text-muted-foreground">· {g.reason}</span>
              </p>
              <p className="text-xs text-muted-foreground">
                by {g.granted_by_name ?? "—"} · {new Date(g.created_at).toLocaleString()}
              </p>
            </div>
            <button
              onClick={() => remove.mutate(g.id)}
              disabled={remove.isPending}
              className="shrink-0 rounded-md bg-red-600 px-2.5 py-1 text-xs font-medium text-white hover:bg-red-700 disabled:opacity-50"
            >
              Remove
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
