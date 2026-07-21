"use client";

import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  adjustLeaveAllocation,
  allocateLeave,
  approveLeave,
  createHoliday,
  createLeaveType,
  deleteLeaveAllocation,
  fetchHolidays,
  fetchLeaveTypes,
  fetchPendingLeave,
  fetchUserLeaveBalance,
  listUsers,
  rejectLeave,
  updateLeaveType,
  type LeaveType,
  type LeaveBalance,
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
    <main className="container mx-auto flex max-w-3xl flex-col gap-6 py-8 sm:py-12">
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

/** HR-only: leave types (with per-category defaults), per-employee allocation
 *  management, and the holiday calendar. */
function HrConfig() {
  const qc = useQueryClient();
  const types = useQuery({ queryKey: ["leave_types"], queryFn: fetchLeaveTypes });
  const users = useQuery({ queryKey: ["users"], queryFn: listUsers });
  const year = new Date().getFullYear();
  const holidays = useQuery({ queryKey: ["holidays", year], queryFn: () => fetchHolidays(year) });

  // Create leave type
  const [typeForm, setTypeForm] = useState({
    name: "",
    paid: true,
    default_days: 0,
    default_days_contractor: 0,
    default_days_intern: 0,
  });
  const addType = useMutation({
    mutationFn: () =>
      createLeaveType({
        name: typeForm.name.trim(),
        paid: typeForm.paid,
        default_days: Number(typeForm.default_days) || 0,
        default_days_contractor: Number(typeForm.default_days_contractor) || 0,
        default_days_intern: Number(typeForm.default_days_intern) || 0,
      }),
    onSuccess: () => {
      setTypeForm({
        name: "",
        paid: true,
        default_days: 0,
        default_days_contractor: 0,
        default_days_intern: 0,
      });
      qc.invalidateQueries({ queryKey: ["leave_types"] });
    },
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
      <Section title="Leave types & category defaults">
        <p className="mb-3 text-xs text-muted-foreground">
          Default days granted per employment category. Employees, project managers and HR use the
          Employee value; contractors and interns use their own. Per-person overrides are set below.
        </p>
        <ul className="mb-4 flex flex-col gap-2">
          {types.data?.map((t) => (
            <LeaveTypeRow key={t.id} type={t} inputClass={input} />
          ))}
          {types.data?.length === 0 && (
            <li className="text-sm text-muted-foreground">None yet.</li>
          )}
        </ul>
        <form
          className="flex flex-wrap items-end gap-3 border-t pt-4"
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
            <span className="text-muted-foreground">Employee days</span>
            <input
              type="number"
              min={0}
              step="0.5"
              value={typeForm.default_days}
              onChange={(e) => setTypeForm({ ...typeForm, default_days: Number(e.target.value) })}
              className={`${input} w-24`}
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Contractor days</span>
            <input
              type="number"
              min={0}
              step="0.5"
              value={typeForm.default_days_contractor}
              onChange={(e) =>
                setTypeForm({ ...typeForm, default_days_contractor: Number(e.target.value) })
              }
              className={`${input} w-24`}
            />
          </label>
          <label className="flex flex-col gap-1 text-xs">
            <span className="text-muted-foreground">Intern days</span>
            <input
              type="number"
              min={0}
              step="0.5"
              value={typeForm.default_days_intern}
              onChange={(e) =>
                setTypeForm({ ...typeForm, default_days_intern: Number(e.target.value) })
              }
              className={`${input} w-24`}
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

      <Section title={`Allocations (${year})`}>
        <AllocationManager users={users.data ?? []} year={year} inputClass={input} />
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

/** One leave type row with an inline editor for its per-category defaults. */
function LeaveTypeRow({ type, inputClass }: { type: LeaveType; inputClass: string }) {
  const qc = useQueryClient();
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState({
    paid: type.paid,
    default_days: type.default_days,
    default_days_contractor: type.default_days_contractor,
    default_days_intern: type.default_days_intern,
  });
  const save = useMutation({
    mutationFn: () =>
      updateLeaveType(type.id, {
        paid: form.paid,
        default_days: Number(form.default_days) || 0,
        default_days_contractor: Number(form.default_days_contractor) || 0,
        default_days_intern: Number(form.default_days_intern) || 0,
      }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["leave_types"] });
      setEditing(false);
    },
  });

  if (!editing) {
    return (
      <li className="flex flex-wrap items-center justify-between gap-2 rounded-md border p-2.5 text-sm">
        <span>
          <span className="font-medium">{type.name}</span>{" "}
          <span className="text-xs text-muted-foreground">
            · Emp {type.default_days}d · Contractor {type.default_days_contractor}d · Intern{" "}
            {type.default_days_intern}d · {type.paid ? "paid" : "unpaid"}
          </span>
        </span>
        <button
          onClick={() => {
            setForm({
              paid: type.paid,
              default_days: type.default_days,
              default_days_contractor: type.default_days_contractor,
              default_days_intern: type.default_days_intern,
            });
            setEditing(true);
          }}
          className="text-xs underline hover:opacity-80"
        >
          Edit
        </button>
      </li>
    );
  }

  return (
    <li className="flex flex-wrap items-end gap-3 rounded-md border p-2.5">
      <span className="text-sm font-medium">{type.name}</span>
      <label className="flex flex-col gap-1 text-xs">
        <span className="text-muted-foreground">Employee</span>
        <input
          type="number"
          min={0}
          step="0.5"
          value={form.default_days}
          onChange={(e) => setForm({ ...form, default_days: Number(e.target.value) })}
          className={`${inputClass} w-20`}
        />
      </label>
      <label className="flex flex-col gap-1 text-xs">
        <span className="text-muted-foreground">Contractor</span>
        <input
          type="number"
          min={0}
          step="0.5"
          value={form.default_days_contractor}
          onChange={(e) => setForm({ ...form, default_days_contractor: Number(e.target.value) })}
          className={`${inputClass} w-20`}
        />
      </label>
      <label className="flex flex-col gap-1 text-xs">
        <span className="text-muted-foreground">Intern</span>
        <input
          type="number"
          min={0}
          step="0.5"
          value={form.default_days_intern}
          onChange={(e) => setForm({ ...form, default_days_intern: Number(e.target.value) })}
          className={`${inputClass} w-20`}
        />
      </label>
      <label className="flex items-center gap-2 text-xs">
        <input
          type="checkbox"
          checked={form.paid}
          onChange={(e) => setForm({ ...form, paid: e.target.checked })}
        />
        Paid
      </label>
      <button
        onClick={() => save.mutate()}
        disabled={save.isPending}
        className="rounded-md bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
      >
        {save.isPending ? "Saving…" : "Save"}
      </button>
      <button onClick={() => setEditing(false)} className="text-xs underline">
        Cancel
      </button>
    </li>
  );
}

/** Pick an employee and set / adjust / reset their per-type allotments.
 *  Effective values fall back to the category default (flagged accordingly). */
function AllocationManager({
  users,
  year,
  inputClass,
}: {
  users: { id: string; name: string }[];
  year: number;
  inputClass: string;
}) {
  const qc = useQueryClient();
  const [userId, setUserId] = useState("");
  const balances = useQuery({
    queryKey: ["user_leave_balance", userId, year],
    queryFn: () => fetchUserLeaveBalance(userId, year),
    enabled: !!userId,
  });
  const invalidate = () =>
    qc.invalidateQueries({ queryKey: ["user_leave_balance", userId, year] });

  const setDays = useMutation({
    mutationFn: (v: { leave_type_id: string; allotted_days: number }) =>
      allocateLeave({ user_id: userId, leave_type_id: v.leave_type_id, year, allotted_days: v.allotted_days }),
    onSuccess: invalidate,
  });
  const adjust = useMutation({
    mutationFn: (v: { leave_type_id: string; delta: number }) =>
      adjustLeaveAllocation({ user_id: userId, leave_type_id: v.leave_type_id, year, delta: v.delta }),
    onSuccess: invalidate,
  });
  const reset = useMutation({
    mutationFn: (leave_type_id: string) =>
      deleteLeaveAllocation({ user_id: userId, leave_type_id, year }),
    onSuccess: invalidate,
  });
  const busy = setDays.isPending || adjust.isPending || reset.isPending;

  return (
    <div className="flex flex-col gap-4">
      <label className="flex max-w-xs flex-col gap-1 text-xs">
        <span className="text-muted-foreground">Employee</span>
        <select value={userId} onChange={(e) => setUserId(e.target.value)} className={inputClass}>
          <option value="">Select…</option>
          {users.map((u) => (
            <option key={u.id} value={u.id}>
              {u.name}
            </option>
          ))}
        </select>
      </label>

      {userId && balances.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
      {userId && balances.data && balances.data.balances.length === 0 && (
        <p className="text-sm text-muted-foreground">No leave types defined yet.</p>
      )}
      {userId && balances.data && balances.data.balances.length > 0 && (
        <div className="overflow-x-auto">
          <table className="w-full min-w-[34rem] text-sm">
            <thead>
              <tr className="border-b text-left text-muted-foreground">
                <th className="py-2 font-medium">Type</th>
                <th className="py-2 font-medium">Allotted</th>
                <th className="py-2 font-medium">Used</th>
                <th className="py-2 font-medium">Remaining</th>
                <th className="py-2 font-medium">Adjust / set</th>
              </tr>
            </thead>
            <tbody>
              {balances.data.balances.map((b) => (
                <AllocationRow
                  key={b.leave_type_id}
                  b={b}
                  inputClass={inputClass}
                  busy={busy}
                  onSet={(days) => setDays.mutate({ leave_type_id: b.leave_type_id, allotted_days: days })}
                  onAdjust={(delta) => adjust.mutate({ leave_type_id: b.leave_type_id, delta })}
                  onReset={() => reset.mutate(b.leave_type_id)}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
      {(setDays.isError || adjust.isError || reset.isError) && (
        <p className="text-sm text-red-600">
          {((setDays.error || adjust.error || reset.error) as Error).message}
        </p>
      )}
    </div>
  );
}

function AllocationRow({
  b,
  inputClass,
  busy,
  onSet,
  onAdjust,
  onReset,
}: {
  b: LeaveBalance;
  inputClass: string;
  busy: boolean;
  onSet: (days: number) => void;
  onAdjust: (delta: number) => void;
  onReset: () => void;
}) {
  const [val, setVal] = useState(String(b.allotted_days));
  // Re-sync the input when the underlying allotment changes (after a mutation).
  useEffect(() => setVal(String(b.allotted_days)), [b.allotted_days]);

  return (
    <tr className="border-b last:border-0">
      <td className="py-2">
        {b.leave_type_name}{" "}
        <span
          className={`ml-1 rounded px-1.5 py-0.5 text-[10px] ${
            b.is_override ? "bg-amber-100 text-amber-800" : "bg-secondary text-muted-foreground"
          }`}
        >
          {b.is_override ? "override" : "default"}
        </span>
      </td>
      <td className="py-2 tabular-nums">{b.allotted_days}</td>
      <td className="py-2 tabular-nums">{b.used_days}</td>
      <td className="py-2 tabular-nums">{b.remaining_days}</td>
      <td className="py-2">
        <div className="flex items-center gap-1.5">
          <button
            onClick={() => onAdjust(-1)}
            disabled={busy}
            className="rounded bg-secondary px-2 py-1 text-xs disabled:opacity-50"
          >
            −1
          </button>
          <button
            onClick={() => onAdjust(1)}
            disabled={busy}
            className="rounded bg-secondary px-2 py-1 text-xs disabled:opacity-50"
          >
            +1
          </button>
          <input
            type="number"
            min={0}
            step="0.5"
            value={val}
            onChange={(e) => setVal(e.target.value)}
            className={`${inputClass} w-20`}
          />
          <button
            onClick={() => onSet(Number(val) || 0)}
            disabled={busy}
            className="rounded bg-primary px-2 py-1 text-xs font-medium text-primary-foreground disabled:opacity-50"
          >
            Set
          </button>
          {b.is_override && (
            <button onClick={onReset} disabled={busy} className="text-xs underline disabled:opacity-50">
              Reset
            </button>
          )}
        </div>
      </td>
    </tr>
  );
}
