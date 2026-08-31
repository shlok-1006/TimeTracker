"use client";

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

type Task = {
  id: string;
  title: string;
  description: string;
  status: string;
  weight: number;
  due_date: string | null;
  /** Full GitHub PR URLs the HRMS performance engine reviews to score the work. */
  pr_links: string[];
  created_at: string;
  updated_at: string;
  /** True when the employee created this task themselves (vs HR/PM-assigned). */
  self_created?: boolean;
};

/** Split a free-text PR box (one URL per line, or comma/space separated) into a
 *  clean list of full URLs. Non-URL junk is dropped so the engine never sees it. */
function parsePrLinks(text: string): string[] {
  return Array.from(
    new Set(
      text
        .split(/[\s,]+/)
        .map((s) => s.trim())
        .filter((s) => s.startsWith("http://") || s.startsWith("https://")),
    ),
  );
}

/** "…/pull/42" → "#42"; otherwise the last path segment. A compact chip label. */
function prLabel(url: string): string {
  const m = url.match(/\/pull\/(\d+)/);
  if (m) return `#${m[1]}`;
  try {
    const parts = new URL(url).pathname.split("/").filter(Boolean);
    return parts.length ? parts[parts.length - 1] : url;
  } catch {
    return url;
  }
}

/** "2026-07-25" → "Jul 25, 2026" (dates are calendar days, no timezone). */
function fmtDue(day: string): string {
  const [y, m, d] = day.split("-").map(Number);
  if (!y || !m || !d) return day;
  return new Date(y, m - 1, d).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/** A due date is overdue if it's before today and the task is still open. */
function isOverdue(day: string, status: string): boolean {
  if (status === "done") return false;
  const today = new Date();
  const todayStr = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(
    today.getDate(),
  ).padStart(2, "0")}`;
  return day < todayStr;
}

/** Tasks the employee is working on: assigned by HR/PM, or self-assigned here.
 *  These feed the AI analysis as work context (like tickets), never Linear. */
export function MyTasks() {
  const qc = useQueryClient();
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [weight, setWeight] = useState(5);
  const [due, setDue] = useState("");
  const [prs, setPrs] = useState("");

  // Inline PR/weight/due editor state — which task is open, and its draft values.
  const [editId, setEditId] = useState<string | null>(null);
  const [editPrs, setEditPrs] = useState("");
  const [editWeight, setEditWeight] = useState(5);
  const [editDue, setEditDue] = useState("");

  function openEditor(t: Task) {
    setEditId(t.id);
    setEditPrs((t.pr_links ?? []).join("\n"));
    setEditWeight(t.weight);
    setEditDue(t.due_date ?? "");
  }

  const tasks = useQuery({
    queryKey: ["me_tasks"],
    queryFn: async () => (await invoker())<Task[]>("me_tasks"),
    refetchInterval: 60_000,
  });

  const invalidate = () => qc.invalidateQueries({ queryKey: ["me_tasks"] });

  const add = useMutation({
    mutationFn: async () => {
      const inv = await invoker();
      return inv("create_my_task", {
        title: title.trim(),
        description: description.trim() || null,
        weight,
        dueDate: due || null,
        prLinks: parsePrLinks(prs),
      });
    },
    onSuccess: () => {
      setTitle("");
      setDescription("");
      setWeight(5);
      setDue("");
      setPrs("");
      invalidate();
    },
  });

  const edit = useMutation({
    mutationFn: async (id: string) => {
      const inv = await invoker();
      return inv("update_my_task", {
        id,
        weight: editWeight,
        dueDate: editDue || null,
        prLinks: parsePrLinks(editPrs),
      });
    },
    onSuccess: () => {
      setEditId(null);
      invalidate();
    },
  });

  const toggle = useMutation({
    mutationFn: async (v: { id: string; status: string }) => {
      const inv = await invoker();
      return inv("set_my_task_status", { id: v.id, status: v.status });
    },
    onSuccess: invalidate,
  });

  const remove = useMutation({
    mutationFn: async (id: string) => {
      const inv = await invoker();
      return inv("delete_my_task", { id });
    },
    onSuccess: invalidate,
  });

  const inputCls =
    "rounded-md border border-slate-200 bg-transparent px-3 py-2 text-sm dark:border-slate-700";

  return (
    <section className="flex flex-col gap-3 rounded-lg border border-slate-200 p-6 dark:border-slate-800">
      <h2 className="font-semibold">Your tasks</h2>

      {tasks.isLoading && <p className="text-sm text-slate-500">Loading…</p>}
      {tasks.error && (
        <p className="text-sm text-red-600">
          {tasks.error instanceof Error ? tasks.error.message : String(tasks.error)}
        </p>
      )}
      {tasks.data && tasks.data.length === 0 && (
        <p className="rounded-md bg-slate-50 p-3 text-sm text-slate-500 dark:bg-slate-800/40">
          No tasks yet — add one below.
        </p>
      )}

      <ul className="flex flex-col gap-2">
        {tasks.data?.map((t) => (
          <li
            key={t.id}
            className="flex flex-col gap-2 rounded-md border border-slate-200 p-3 dark:border-slate-700"
          >
            <div className="flex items-start justify-between gap-3">
              <div className={t.status === "done" ? "opacity-60" : ""}>
                <p className={`font-medium ${t.status === "done" ? "line-through" : ""}`}>
                  {t.title}
                </p>
                {t.description && <p className="text-sm text-slate-500">{t.description}</p>}
                <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-slate-500">
                  <span className="rounded bg-slate-100 px-1.5 py-0.5 font-medium text-slate-600 dark:bg-slate-800 dark:text-slate-300">
                    Weight {t.weight}/10
                  </span>
                  {t.due_date && (
                    <span
                      className={
                        isOverdue(t.due_date, t.status)
                          ? "font-medium text-red-600"
                          : "text-slate-500"
                      }
                    >
                      Due {fmtDue(t.due_date)}
                      {isOverdue(t.due_date, t.status) ? " · overdue" : ""}
                    </span>
                  )}
                </div>
                {/* Linked PRs — the HRMS engine scores the work behind these. */}
                {t.pr_links?.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
                    <span className="text-xs text-slate-400">PRs:</span>
                    {t.pr_links.map((url) => (
                      <a
                        key={url}
                        href={url}
                        target="_blank"
                        rel="noreferrer"
                        title={url}
                        className="rounded bg-violet-50 px-1.5 py-0.5 text-xs font-medium text-violet-700 hover:underline dark:bg-violet-950/40 dark:text-violet-300"
                      >
                        {prLabel(url)}
                      </a>
                    ))}
                  </div>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-2">
                <button
                  type="button"
                  onClick={() =>
                    toggle.mutate({ id: t.id, status: t.status === "done" ? "open" : "done" })
                  }
                  disabled={toggle.isPending}
                  className="rounded-md border border-slate-200 px-2.5 py-1 text-xs font-medium hover:bg-slate-100 disabled:opacity-50 dark:border-slate-700 dark:hover:bg-slate-800"
                >
                  {t.status === "done" ? "Reopen" : "Mark done"}
                </button>
                {t.self_created && (
                  <>
                    <button
                      type="button"
                      onClick={() => (editId === t.id ? setEditId(null) : openEditor(t))}
                      className="rounded-md border border-slate-200 px-2 py-1 text-xs font-medium hover:bg-slate-100 disabled:opacity-50 dark:border-slate-700 dark:hover:bg-slate-800"
                    >
                      {editId === t.id ? "Close" : "Edit PRs"}
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        if (confirm(`Delete "${t.title}"?`)) remove.mutate(t.id);
                      }}
                      disabled={remove.isPending}
                      className="rounded-md border border-slate-200 px-2 py-1 text-xs font-medium text-red-600 hover:bg-red-50 disabled:opacity-50 dark:border-slate-700 dark:hover:bg-red-950/40"
                    >
                      Delete
                    </button>
                  </>
                )}
              </div>
            </div>

            {/* Inline editor: PR links + weight + due (self-created tasks only). */}
            {editId === t.id && (
              <div className="flex flex-col gap-2 rounded-md bg-slate-50 p-3 dark:bg-slate-800/40">
                <label className="text-xs font-medium text-slate-500">
                  PR links (one full GitHub URL per line)
                  <textarea
                    value={editPrs}
                    onChange={(e) => setEditPrs(e.target.value)}
                    rows={3}
                    placeholder="https://github.com/org/repo/pull/42"
                    className={`${inputCls} mt-1 w-full font-mono text-xs`}
                  />
                </label>
                <div className="flex flex-wrap items-center gap-2">
                  <label className="text-xs text-slate-500">
                    Weight{" "}
                    <select
                      value={editWeight}
                      onChange={(e) => setEditWeight(Number(e.target.value))}
                      className="rounded-md border border-slate-200 bg-transparent px-2 py-1 text-sm dark:border-slate-700"
                    >
                      {Array.from({ length: 10 }, (_, i) => i + 1).map((w) => (
                        <option key={w} value={w}>
                          {w}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="text-xs text-slate-500">
                    Due{" "}
                    <input
                      type="date"
                      value={editDue}
                      onChange={(e) => setEditDue(e.target.value)}
                      className="rounded-md border border-slate-200 bg-transparent px-2 py-1 text-sm dark:border-slate-700"
                    />
                  </label>
                  <button
                    type="button"
                    onClick={() => edit.mutate(t.id)}
                    disabled={edit.isPending}
                    className="ml-auto rounded-md bg-slate-900 px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-50 dark:bg-white dark:text-slate-900"
                  >
                    {edit.isPending ? "Saving…" : "Save"}
                  </button>
                  <button
                    type="button"
                    onClick={() => setEditId(null)}
                    className="rounded-md border border-slate-200 px-3 py-1.5 text-xs font-medium hover:bg-slate-100 dark:border-slate-700 dark:hover:bg-slate-800"
                  >
                    Cancel
                  </button>
                </div>
                {edit.error && (
                  <p className="text-xs text-red-600">
                    {edit.error instanceof Error ? edit.error.message : String(edit.error)}
                  </p>
                )}
              </div>
            )}
          </li>
        ))}
      </ul>

      {/* Self-assign a task */}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (title.trim()) add.mutate();
        }}
        className="mt-1 flex flex-col gap-2 rounded-md border border-slate-200 p-3 dark:border-slate-700"
      >
        <p className="text-sm font-medium">Add a task for yourself</p>
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="What are you working on?"
          className={inputCls}
        />
        <input
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Description (optional)"
          className={inputCls}
        />
        <textarea
          value={prs}
          onChange={(e) => setPrs(e.target.value)}
          rows={2}
          placeholder="PR links (optional) — one full GitHub URL per line"
          className={`${inputCls} font-mono text-xs`}
        />
        <div className="flex flex-wrap items-center gap-2">
          <label className="text-xs text-slate-500">
            Weight{" "}
            <select
              value={weight}
              onChange={(e) => setWeight(Number(e.target.value))}
              className="rounded-md border border-slate-200 bg-transparent px-2 py-1 text-sm dark:border-slate-700"
            >
              {Array.from({ length: 10 }, (_, i) => i + 1).map((w) => (
                <option key={w} value={w}>
                  {w}
                </option>
              ))}
            </select>
          </label>
          <label className="text-xs text-slate-500">
            Due{" "}
            <input
              type="date"
              value={due}
              onChange={(e) => setDue(e.target.value)}
              className="rounded-md border border-slate-200 bg-transparent px-2 py-1 text-sm dark:border-slate-700"
            />
          </label>
          <button
            type="submit"
            disabled={add.isPending || !title.trim()}
            className="ml-auto rounded-md bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:opacity-90 disabled:opacity-50 dark:bg-white dark:text-slate-900"
          >
            {add.isPending ? "Adding…" : "Add task"}
          </button>
        </div>
        {add.error && (
          <p className="text-sm text-red-600">
            {add.error instanceof Error ? add.error.message : String(add.error)}
          </p>
        )}
      </form>
    </section>
  );
}
