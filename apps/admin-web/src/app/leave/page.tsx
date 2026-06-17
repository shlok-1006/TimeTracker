"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  allocateLeave,
  approveLeave,
  createHoliday,
  createLeaveType,
  fetchHolidays,
  fetchLeaveTypes,
  fetchPendingLeave,
  listUsers,
  rejectLeave,
} from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border bg-card p-6 text-card-foreground">
      <h2 className="mb-4 text-lg font-semibold">{title}</h2>
      {children}
    </section>
  );
}

export default function LeavePage() {
  const { user, ready } = useAdminSession();
  const qc = useQueryClient();
  const isHr = user?.role === "hr";

  const pending = useQuery({
    queryKey: ["pending_leave"],
    queryFn: fetchPendingLeave,
    enabled: ready,
    refetchInterval: 30_000,
  });

  const decide = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "approve" | "reject" }) =>
      action === "approve" ? approveLeave(id) : rejectLeave(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["pending_leave"] }),
  });

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="container mx-auto flex max-w-3xl flex-col gap-6 py-12">
      <header>
        <h1 className="text-2xl font-bold tracking-tight">Leave</h1>
      </header>

      <Section title="Pending requests">
        {pending.isLoading && <p className="text-muted-foreground">Loading…</p>}
        {pending.error && <p className="text-red-600">{(pending.error as Error).message}</p>}
        {pending.data && pending.data.length === 0 && (
          <p className="text-muted-foreground">No pending requests.</p>
        )}
        {decide.isError && (
          <p className="mb-3 text-sm text-red-600">{(decide.error as Error).message}</p>
        )}
        {pending.data && pending.data.length > 0 && (
          <ul className="flex flex-col gap-3">
            {pending.data.map((r) => (
              <li
                key={r.id}
                className="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3"
              >
                <div>
                  <p className="font-medium">
                    {r.employee_name}{" "}
                    <span className="text-sm font-normal text-muted-foreground">
                      · {r.leave_type_name} · {r.days} day{r.days === 1 ? "" : "s"}
                    </span>
                  </p>
                  <p className="text-xs text-muted-foreground">
                    {r.start_date} → {r.end_date}
                    {r.reason ? ` · ${r.reason}` : ""}
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => decide.mutate({ id: r.id, action: "approve" })}
                    disabled={decide.isPending}
                    className="rounded-md bg-green-600 px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
                  >
                    Approve
                  </button>
                  <button
                    onClick={() => decide.mutate({ id: r.id, action: "reject" })}
                    disabled={decide.isPending}
                    className="rounded-md bg-red-600 px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50"
                  >
                    Reject
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Section>

      {isHr && <HrConfig />}
    </main>
  );
}

/** HR-only: leave types, per-employee allocations, and the holiday calendar. */
function HrConfig() {
  const qc = useQueryClient();
  const types = useQuery({ queryKey: ["leave_types"], queryFn: fetchLeaveTypes });
  const users = useQuery({ queryKey: ["users"], queryFn: listUsers });
  const year = new Date().getFullYear();
  const holidays = useQuery({ queryKey: ["holidays", year], queryFn: () => fetchHolidays(year) });

  // Create leave type
  const [typeForm, setTypeForm] = useState({ name: "", paid: true, default_days: 0 });
  const addType = useMutation({
    mutationFn: () =>
      createLeaveType({
        name: typeForm.name.trim(),
        paid: typeForm.paid,
        default_days: Number(typeForm.default_days) || 0,
      }),
    onSuccess: () => {
      setTypeForm({ name: "", paid: true, default_days: 0 });
      qc.invalidateQueries({ queryKey: ["leave_types"] });
    },
  });

  // Allocate
  const [alloc, setAlloc] = useState({ user_id: "", leave_type_id: "", allotted_days: 0 });
  const allocate = useMutation({
    mutationFn: () =>
      allocateLeave({
        user_id: alloc.user_id,
        leave_type_id: alloc.leave_type_id,
        allotted_days: Number(alloc.allotted_days) || 0,
      }),
    onSuccess: () => setAlloc({ user_id: "", leave_type_id: "", allotted_days: 0 }),
  });

  // Holidays
  const [holiday, setHoliday] = useState({ day: "", name: "" });
  const addHoliday = useMutation({
    mutationFn: () => createHoliday(holiday.day, holiday.name.trim()),
    onSuccess: () => {
      setHoliday({ day: "", name: "" });
      qc.invalidateQueries({ queryKey: ["holidays", year] });
    },
  });

  const input = "rounded-md border border-input bg-background px-3 py-1.5 text-sm";

  return (
    <>
      <Section title="Leave types">
        <ul className="mb-4 flex flex-wrap gap-2">
          {types.data?.map((t) => (
            <li key={t.id} className="rounded-full bg-secondary px-3 py-1 text-xs">
              {t.name} · {t.default_days}d · {t.paid ? "paid" : "unpaid"}
            </li>
          ))}
          {types.data?.length === 0 && (
            <li className="text-sm text-muted-foreground">None yet.</li>
          )}
        </ul>
        <form
          className="flex flex-wrap items-end gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (typeForm.name.trim()) addType.mutate();
          }}
        >
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Name</span>
            <input
              value={typeForm.name}
              onChange={(e) => setTypeForm({ ...typeForm, name: e.target.value })}
              className={input}
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Default days</span>
            <input
              type="number"
              min={0}
              step="0.5"
              value={typeForm.default_days}
              onChange={(e) => setTypeForm({ ...typeForm, default_days: Number(e.target.value) })}
              className={`${input} w-28`}
            />
          </label>
          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={typeForm.paid}
              onChange={(e) => setTypeForm({ ...typeForm, paid: e.target.checked })}
            />
            Paid
          </label>
          <button
            type="submit"
            disabled={addType.isPending}
            className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
          >
            Add type
          </button>
        </form>
        {addType.isError && (
          <p className="mt-2 text-sm text-red-600">{(addType.error as Error).message}</p>
        )}
      </Section>

      <Section title={`Allocate days (${year})`}>
        <form
          className="flex flex-wrap items-end gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (alloc.user_id && alloc.leave_type_id) allocate.mutate();
          }}
        >
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Employee</span>
            <select
              value={alloc.user_id}
              onChange={(e) => setAlloc({ ...alloc, user_id: e.target.value })}
              className={input}
            >
              <option value="">Select…</option>
              {users.data?.map((u) => (
                <option key={u.id} value={u.id}>
                  {u.name}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Type</span>
            <select
              value={alloc.leave_type_id}
              onChange={(e) => setAlloc({ ...alloc, leave_type_id: e.target.value })}
              className={input}
            >
              <option value="">Select…</option>
              {types.data?.map((t) => (
                <option key={t.id} value={t.id}>
                  {t.name}
                </option>
              ))}
            </select>
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Days</span>
            <input
              type="number"
              min={0}
              step="0.5"
              value={alloc.allotted_days}
              onChange={(e) => setAlloc({ ...alloc, allotted_days: Number(e.target.value) })}
              className={`${input} w-28`}
            />
          </label>
          <button
            type="submit"
            disabled={allocate.isPending}
            className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
          >
            Allocate
          </button>
        </form>
        {allocate.isError && (
          <p className="mt-2 text-sm text-red-600">{(allocate.error as Error).message}</p>
        )}
        {allocate.isSuccess && <p className="mt-2 text-sm text-green-600">Allocation saved.</p>}
      </Section>

      <Section title={`Holidays (${year})`}>
        <ul className="mb-4 flex flex-col gap-1">
          {holidays.data?.map((h) => (
            <li key={h.id} className="text-sm">
              <span className="tabular-nums text-muted-foreground">{h.day}</span> — {h.name}
            </li>
          ))}
          {holidays.data?.length === 0 && (
            <li className="text-sm text-muted-foreground">No holidays set.</li>
          )}
        </ul>
        <form
          className="flex flex-wrap items-end gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (holiday.day && holiday.name.trim()) addHoliday.mutate();
          }}
        >
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Date</span>
            <input
              type="date"
              value={holiday.day}
              onChange={(e) => setHoliday({ ...holiday, day: e.target.value })}
              className={input}
            />
          </label>
          <label className="flex flex-1 flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Name</span>
            <input
              value={holiday.name}
              onChange={(e) => setHoliday({ ...holiday, name: e.target.value })}
              className={input}
            />
          </label>
          <button
            type="submit"
            disabled={addHoliday.isPending}
            className="rounded-md bg-primary px-4 py-1.5 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
          >
            Add holiday
          </button>
        </form>
        {addHoliday.isError && (
          <p className="mt-2 text-sm text-red-600">{(addHoliday.error as Error).message}</p>
        )}
      </Section>
    </>
  );
}
