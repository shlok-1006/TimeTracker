"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

type LeaveType = { id: string; name: string; paid: boolean; default_days: number };
type Balance = {
  leave_type_id: string;
  leave_type_name: string;
  paid: boolean;
  allotted_days: number;
  used_days: number;
  remaining_days: number;
};
type BalanceResp = { year: number; balances: Balance[] };
type LeaveRequest = {
  id: string;
  leave_type_name: string;
  start_date: string;
  end_date: string;
  days: number;
  reason: string;
  status: string;
  created_at: string;
};

const STATUS_BADGE: Record<string, string> = {
  pending: "bg-amber-100 text-amber-800",
  approved: "bg-green-100 text-green-800",
  rejected: "bg-red-100 text-red-800",
  cancelled: "bg-slate-100 text-slate-600",
};

async function call<T>(cmd: string, args?: Record<string, unknown>) {
  return (await invoker())<T>(cmd, args);
}

/** Employee leave self-service: balances, apply for leave, request history. */
export function MyLeave() {
  const qc = useQueryClient();

  const types = useQuery({
    queryKey: ["me_leave_types"],
    queryFn: () => call<LeaveType[]>("me_leave_types"),
  });
  const balance = useQuery({
    queryKey: ["me_leave_balance"],
    queryFn: () => call<BalanceResp>("me_leave_balance"),
  });
  const requests = useQuery({
    queryKey: ["me_leave_requests"],
    queryFn: () => call<LeaveRequest[]>("me_leave_requests"),
    refetchInterval: 60_000,
  });

  const today = new Date().toLocaleDateString("en-CA");
  const [form, setForm] = useState({ leave_type_id: "", start_date: today, end_date: today, reason: "" });

  // The effective selected type: the explicit choice, else the first available
  // type (which is what the <select> shows by default). Submitting must use
  // THIS — not the raw form state — so an untouched dropdown still sends a valid
  // leave_type_id instead of "" (which the server rejects with 422).
  const typeId = form.leave_type_id || types.data?.[0]?.id || "";

  const refresh = () => {
    qc.invalidateQueries({ queryKey: ["me_leave_requests"] });
    qc.invalidateQueries({ queryKey: ["me_leave_balance"] });
  };

  const apply = useMutation({
    mutationFn: () =>
      call("request_leave", {
        leaveTypeId: typeId,
        startDate: form.start_date,
        endDate: form.end_date,
        reason: form.reason.trim(),
      }),
    onSuccess: () => {
      setForm((f) => ({ ...f, reason: "" }));
      refresh();
    },
  });
  const cancel = useMutation({
    mutationFn: (id: string) => call("cancel_leave", { id }),
    onSuccess: refresh,
  });

  return (
    <section className="flex flex-col gap-4 rounded-lg border border-slate-200 p-6 dark:border-slate-800">
      <h2 className="font-semibold">Leave</h2>

      {/* Balances */}
      {balance.isLoading && <p className="text-sm text-slate-500">Loading…</p>}
      {balance.error && (
        <p className="text-sm text-red-600">
          {balance.error instanceof Error ? balance.error.message : String(balance.error)}
        </p>
      )}
      {balance.data && (
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
          {balance.data.balances.map((b) => (
            <div
              key={b.leave_type_id}
              className="rounded-md border border-slate-200 p-3 dark:border-slate-700"
            >
              <div className="flex items-center justify-between">
                <span className="font-medium">{b.leave_type_name}</span>
                <span className="text-xs text-slate-500">{b.paid ? "paid" : "unpaid"}</span>
              </div>
              <div className="mt-1 text-sm text-slate-600 dark:text-slate-300">
                <span className="font-semibold tabular-nums">{b.remaining_days}</span> left
                <span className="text-slate-400">
                  {" "}
                  · {b.used_days} used of {b.allotted_days}
                </span>
              </div>
            </div>
          ))}
          {balance.data.balances.length === 0 && (
            <p className="text-sm text-slate-500">No leave types configured yet.</p>
          )}
        </div>
      )}

      {/* Apply */}
      <form
        className="flex flex-wrap items-end gap-3 border-t border-slate-100 pt-4 dark:border-slate-800"
        onSubmit={(e) => {
          e.preventDefault();
          if (typeId) apply.mutate();
        }}
      >
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-slate-500">Type</span>
          <select
            value={typeId}
            onChange={(e) => setForm({ ...form, leave_type_id: e.target.value })}
            className="rounded-md border border-slate-300 bg-transparent px-2 py-1.5 text-sm dark:border-slate-700"
          >
            {types.data?.map((t) => (
              <option key={t.id} value={t.id}>
                {t.name}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-slate-500">From</span>
          <input
            type="date"
            value={form.start_date}
            onChange={(e) => setForm({ ...form, start_date: e.target.value })}
            className="rounded-md border border-slate-300 bg-transparent px-2 py-1.5 text-sm dark:border-slate-700"
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-slate-500">To</span>
          <input
            type="date"
            value={form.end_date}
            onChange={(e) => setForm({ ...form, end_date: e.target.value })}
            className="rounded-md border border-slate-300 bg-transparent px-2 py-1.5 text-sm dark:border-slate-700"
          />
        </label>
        <label className="flex flex-1 flex-col gap-1 text-xs">
          <span className="text-slate-500">Reason</span>
          <input
            value={form.reason}
            onChange={(e) => setForm({ ...form, reason: e.target.value })}
            placeholder="optional"
            className="rounded-md border border-slate-300 bg-transparent px-2 py-1.5 text-sm dark:border-slate-700"
          />
        </label>
        <button
          type="submit"
          disabled={apply.isPending || !typeId}
          className="rounded-md bg-purple-600 px-4 py-1.5 text-sm font-medium text-white hover:bg-purple-700 disabled:opacity-50"
        >
          {apply.isPending ? "Applying…" : "Apply"}
        </button>
      </form>
      {apply.isError && (
        <p className="text-sm text-red-600">
          {apply.error instanceof Error ? apply.error.message : String(apply.error)}
        </p>
      )}

      {/* History */}
      <div className="flex flex-col gap-2">
        <h3 className="text-sm font-semibold text-slate-600 dark:text-slate-300">My requests</h3>
        {requests.data && requests.data.length === 0 && (
          <p className="text-sm text-slate-500">No leave requests yet.</p>
        )}
        <ul className="flex flex-col gap-2">
          {requests.data?.map((r) => (
            <li
              key={r.id}
              className="flex items-center justify-between gap-3 rounded-md border border-slate-200 p-3 text-sm dark:border-slate-700"
            >
              <div>
                <p className="font-medium">
                  {r.leave_type_name} · {r.days} day{r.days === 1 ? "" : "s"}
                </p>
                <p className="text-xs text-slate-500">
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
      </div>
    </section>
  );
}
