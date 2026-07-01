"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  cancelLeaveRequest,
  fetchLeaveTypes,
  fetchMyLeaveBalance,
  fetchMyLeaveRequests,
  requestLeave,
} from "@/lib/api";
import { useEmployeeSession } from "@/components/use-employee-session";

const STATUS_BADGE: Record<string, string> = {
  pending: "bg-amber-100 text-amber-800",
  approved: "bg-green-100 text-green-800",
  rejected: "bg-red-100 text-red-800",
  cancelled: "bg-slate-100 text-slate-600",
};

export default function LeavePage() {
  const { ready } = useEmployeeSession();
  const qc = useQueryClient();

  const types = useQuery({ queryKey: ["leave_types"], queryFn: fetchLeaveTypes, enabled: ready });
  const balance = useQuery({
    queryKey: ["my_leave_balance"],
    queryFn: () => fetchMyLeaveBalance(),
    enabled: ready,
  });
  const requests = useQuery({
    queryKey: ["my_leave_requests"],
    queryFn: fetchMyLeaveRequests,
    refetchInterval: 60_000,
    enabled: ready,
  });

  const today = new Date().toLocaleDateString("en-CA");
  const [form, setForm] = useState({ leave_type_id: "", start_date: today, end_date: today, reason: "" });

  const refresh = () => {
    qc.invalidateQueries({ queryKey: ["my_leave_requests"] });
    qc.invalidateQueries({ queryKey: ["my_leave_balance"] });
  };

  const apply = useMutation({
    mutationFn: () =>
      requestLeave({
        leave_type_id: form.leave_type_id,
        start_date: form.start_date,
        end_date: form.end_date,
        reason: form.reason.trim(),
      }),
    onSuccess: () => {
      setForm((f) => ({ ...f, reason: "" }));
      refresh();
    },
  });
  const cancel = useMutation({
    mutationFn: (id: string) => cancelLeaveRequest(id),
    onSuccess: refresh,
  });

  const typeId = form.leave_type_id || types.data?.[0]?.id || "";

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="container mx-auto flex max-w-4xl flex-col gap-6 py-12">
      <header>
        <h1 className="text-2xl font-bold tracking-tight">Leave</h1>
        <p className="text-muted-foreground">Balances, requests, and history.</p>
      </header>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-3 text-lg font-semibold">Balance</h2>
        {balance.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
        {balance.error && <p className="text-sm text-red-600">{(balance.error as Error).message}</p>}
        {balance.data && (
          <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
            {balance.data.balances.map((b) => (
              <div key={b.leave_type_id} className="rounded-md border p-3">
                <div className="flex items-center justify-between">
                  <span className="font-medium">{b.leave_type_name}</span>
                  <span className="text-xs text-muted-foreground">{b.paid ? "paid" : "unpaid"}</span>
                </div>
                <div className="mt-1 text-sm text-muted-foreground">
                  <span className="font-semibold tabular-nums text-foreground">
                    {b.remaining_days}
                  </span>{" "}
                  left · {b.used_days} used of {b.allotted_days}
                </div>
              </div>
            ))}
            {balance.data.balances.length === 0 && (
              <p className="text-sm text-muted-foreground">No leave types configured yet.</p>
            )}
          </div>
        )}
      </section>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-3 text-lg font-semibold">Apply for leave</h2>
        <form
          className="flex flex-wrap items-end gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (typeId) apply.mutate();
          }}
        >
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Type</span>
            <select
              value={typeId}
              onChange={(e) => setForm({ ...form, leave_type_id: e.target.value })}
              className="rounded-md border border-input bg-background px-2 py-1.5 text-sm"
            >
              {types.data?.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">From</span>
            <input
              type="date"
              value={form.start_date}
              onChange={(e) => setForm({ ...form, start_date: e.target.value })}
              className="rounded-md border border-input bg-background px-2 py-1.5 text-sm"
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">To</span>
            <input
              type="date"
              value={form.end_date}
              onChange={(e) => setForm({ ...form, end_date: e.target.value })}
              className="rounded-md border border-input bg-background px-2 py-1.5 text-sm"
            />
          </label>
          <label className="flex flex-1 flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Reason</span>
            <input
              value={form.reason}
              onChange={(e) => setForm({ ...form, reason: e.target.value })}
              placeholder="optional"
              className="rounded-md border border-input bg-background px-2 py-1.5 text-sm"
            />
          </label>
          <button
            type="submit"
            disabled={apply.isPending || !typeId}
            className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
          >
            {apply.isPending ? "Applying…" : "Apply"}
          </button>
        </form>
        {apply.isError && (
          <p className="mt-2 text-sm text-red-600">
            {apply.error instanceof Error ? apply.error.message : String(apply.error)}
          </p>
        )}
      </section>

      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-3 text-lg font-semibold">My requests</h2>
        {requests.data && requests.data.length === 0 && (
          <p className="text-sm text-muted-foreground">No leave requests yet.</p>
        )}
        <ul className="flex flex-col gap-2">
          {requests.data?.map((r) => (
            <li
              key={r.id}
              className="flex items-center justify-between gap-3 rounded-md border p-3 text-sm"
            >
              <div>
                <p className="font-medium">
                  {r.leave_type_name} · {r.days} day{r.days === 1 ? "" : "s"}
                </p>
                <p className="text-xs text-muted-foreground">
                  {r.start_date} → {r.end_date}
                  {r.reason ? ` · ${r.reason}` : ""}
                </p>
              </div>
              <div className="flex items-center gap-2">
                <span
                  className={`rounded-full px-2 py-0.5 text-[11px] font-medium ${
                    STATUS_BADGE[r.status] ?? "bg-slate-100 text-slate-600"
                  }`}
                >
                  {r.status}
                </span>
                {r.status === "pending" && (
                  <button
                    onClick={() => cancel.mutate(r.id)}
                    disabled={cancel.isPending}
                    className="text-xs text-red-600 hover:underline disabled:opacity-50"
                  >
                    cancel
                  </button>
                )}
              </div>
            </li>
          ))}
        </ul>
      </section>
    </main>
  );
}
