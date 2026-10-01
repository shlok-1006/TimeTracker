"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

type LeaveType = {
  id: string;
  name: string;
  paid: boolean;
  default_days: number;
  eligible_gender?: string | null;
  min_tenure_months?: number;
};
type Balance = {
  leave_type_id: string;
  leave_type_name: string;
  paid: boolean;
  allotted_days: number;
  used_days: number;
  remaining_days: number;
  /** Server 0050+: eligibility rule (paternity/maternity) and how days are counted. Optional so an older
   *  server keeps working (everything treated as eligible, working days). */
  is_override?: boolean;
  eligible?: boolean;
  eligibility_note?: string | null;
  day_basis?: "working" | "calendar";
  special?: boolean;
  /** Set when only service time is missing: leave may START on/after this date. */
  eligible_from?: string | null;
};
/** A type the person can actually use: eligible, or granted by HR as an allocation. */
const usable = (b: Balance) => b.eligible !== false || b.is_override === true;
/** Offer it: usable, or only short of service time (the server judges eligibility on the start date). */
const offerable = (b: Balance) => usable(b) || !!b.eligible_from;
/** A type with an eligibility rule (paternity / maternity) — hidden when we can't judge it. */
const isSpecial = (t: LeaveType) => !!t.eligible_gender || (t.min_tenure_months ?? 0) > 0;
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
  const [form, setForm] = useState({
    leave_type_id: "",
    start_date: today,
    end_date: today,
    days: "",
    reason: "",
  });

  const refresh = () => {
    qc.invalidateQueries({ queryKey: ["me_leave_requests"] });
    qc.invalidateQueries({ queryKey: ["me_leave_balance"] });
  };

  // Half-leave: a blank box means "whole business days in the range"; a value
  // (0.5, 1, 1.5, …) is sent as the explicit duration. Reject anything that
  // isn't a positive multiple of 0.5 before it reaches the server.
  const daysText = form.days.trim().replace(",", "."); // "1,5" means 1.5
  const parsedDays = daysText === "" || !/^\d+(\.\d+)?$/.test(daysText) ? null : Number(daysText);
  const daysInvalid =
    daysText !== "" &&
    (parsedDays === null || parsedDays <= 0 || (parsedDays * 2) % 1 !== 0 || parsedDays > 366);

  const apply = useMutation({
    // The type is passed in: the select shows a DEFAULT (first usable type) that never touches
    // form.leave_type_id, so reading the form here sent "" whenever the default was kept.
    mutationFn: (leaveTypeId: string) =>
      call("request_leave", {
        leaveTypeId,
        startDate: form.start_date,
        endDate: form.end_date,
        reason: form.reason.trim(),
        days: parsedDays,
      }),
    onSuccess: () => {
      setForm((f) => ({ ...f, reason: "", days: "" })); // a leftover 0.5 must not ride into the next request
      refresh();
    },
  });
  const cancel = useMutation({
    mutationFn: (id: string) => call("cancel_leave", { id }),
    onSuccess: refresh,
  });

  // Only offer types the person can use — a man is never offered maternity leave, and paternity /
  // maternity appear once the 1-year mark is reached. Without balances yet, fall back to every type.
  const usableIds = balance.data ? new Set(balance.data.balances.filter(offerable).map((b) => b.leave_type_id)) : null;
  const offered = (types.data ?? []).filter((t) => (usableIds ? usableIds.has(t.id) : !isSpecial(t)));
  // Default the type select to the first available type.
  const typeId = (offered.some((t) => t.id === form.leave_type_id) ? form.leave_type_id : "") || offered[0]?.id || "";
  const chosen = balance.data?.balances.find((b) => b.leave_type_id === typeId);
  const calendarBasis = chosen?.day_basis === "calendar";

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
          {balance.data.balances.filter(usable).map((b) => (
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
          {balance.data.balances.filter(usable).length === 0 && (
            <p className="text-sm text-slate-500">No leave types configured yet.</p>
          )}
        </div>
      )}

      {/* Apply */}
      <form
        className="flex flex-wrap items-end gap-3 border-t border-slate-100 pt-4 dark:border-slate-800"
        onSubmit={(e) => {
          e.preventDefault();
          if (typeId && !daysInvalid) apply.mutate(typeId);
        }}
      >
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-slate-500">Type</span>
          <select
            value={typeId}
            onChange={(e) => setForm({ ...form, leave_type_id: e.target.value })}
            className="rounded-md border border-slate-300 bg-transparent px-2 py-1.5 text-sm dark:border-slate-700"
          >
            {offered.map((t) => (
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
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-slate-500">Days</span>
          <input
            type="text"
            inputMode="decimal"
            aria-invalid={daysInvalid}
            value={form.days}
            onChange={(e) => setForm({ ...form, days: e.target.value })}
            placeholder="auto"
            title="Leave blank for whole days, or enter 0.5, 1, 1.5, 2.5… for a half/partial leave"
            className={`w-24 rounded-md border bg-transparent px-2 py-1.5 text-sm dark:border-slate-700 ${
              daysInvalid ? "border-red-400" : "border-slate-300"
            }`}
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
          disabled={apply.isPending || !typeId || daysInvalid}
          className="rounded-md bg-purple-600 px-4 py-1.5 text-sm font-medium text-white hover:bg-purple-700 disabled:opacity-50"
        >
          {apply.isPending ? "Applying…" : "Apply"}
        </button>
      </form>
      {chosen && !usable(chosen) && chosen.eligible_from && !(form.start_date >= chosen.eligible_from) && (
        <p className="text-xs text-amber-700">
          {chosen.leave_type_name} is available for leave starting on or after {chosen.eligible_from} (one year of service).
        </p>
      )}
      {calendarBasis && (
        <p className="text-xs text-slate-500">
          {chosen?.leave_type_name} counts every day in the range, weekends and holidays included.
        </p>
      )}
      {daysInvalid && (
        <p className="text-sm text-red-600">
          Days must be a positive multiple of 0.5 (e.g. 0.5, 1, 1.5, 2.5). Leave blank to count
          whole days automatically.
        </p>
      )}
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
