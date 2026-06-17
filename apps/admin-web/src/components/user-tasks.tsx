"use client";

import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createUserTask, deleteTask, fetchUserTasks, setTaskStatus } from "@/lib/api";

/** HR control to assign manual tasks to an employee + list/close/delete them.
 *  These feed the employee's AI analysis context (never touch Linear). */
export function UserTasks({ userId }: { userId: string }) {
  const qc = useQueryClient();
  const tasks = useQuery({ queryKey: ["user_tasks", userId], queryFn: () => fetchUserTasks(userId) });

  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [err, setErr] = useState<string | null>(null);

  const invalidate = () => qc.invalidateQueries({ queryKey: ["user_tasks", userId] });
  const create = useMutation({
    mutationFn: () => createUserTask(userId, title.trim(), description.trim()),
    onSuccess: () => {
      setTitle("");
      setDescription("");
      setErr(null);
      invalidate();
    },
    onError: (e) => setErr(e instanceof Error ? e.message : "Failed to assign task."),
  });
  const toggle = useMutation({
    mutationFn: (t: { id: string; status: string }) =>
      setTaskStatus(t.id, t.status === "open" ? "done" : "open"),
    onSuccess: invalidate,
  });
  const remove = useMutation({ mutationFn: (id: string) => deleteTask(id), onSuccess: invalidate });

  function submit(e: FormEvent) {
    e.preventDefault();
    if (title.trim()) create.mutate();
  }

  return (
    <section className="rounded-lg border bg-card p-6 text-card-foreground">
      <h2 className="mb-4 text-lg font-semibold">Assigned tasks</h2>

      <form onSubmit={submit} className="mb-4 flex flex-col gap-2 sm:flex-row">
        <input
          required
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="Task title"
          className="flex-1 rounded-md border border-input bg-background px-3 py-2 text-sm"
        />
        <input
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Description (optional)"
          className="flex-1 rounded-md border border-input bg-background px-3 py-2 text-sm"
        />
        <button
          type="submit"
          disabled={create.isPending || !title.trim()}
          className="rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50"
        >
          Assign
        </button>
      </form>
      {err && <p className="mb-2 text-sm text-red-600">{err}</p>}

      {tasks.isLoading && <p className="text-sm text-muted-foreground">Loading…</p>}
      {tasks.data && tasks.data.length === 0 && (
        <p className="text-sm text-muted-foreground">No tasks assigned yet.</p>
      )}
      <ul className="flex flex-col gap-2">
        {tasks.data?.map((t) => (
          <li
            key={t.id}
            className="flex items-start justify-between gap-3 rounded-md border p-3"
          >
            <div className={t.status === "done" ? "opacity-60" : ""}>
              <p className={`font-medium ${t.status === "done" ? "line-through" : ""}`}>{t.title}</p>
              {t.description && <p className="text-sm text-muted-foreground">{t.description}</p>}
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <button
                onClick={() => toggle.mutate({ id: t.id, status: t.status })}
                disabled={toggle.isPending}
                className="rounded-md bg-secondary px-2.5 py-1 text-xs font-medium hover:opacity-90 disabled:opacity-50"
              >
                {t.status === "open" ? "Mark done" : "Reopen"}
              </button>
              <button
                onClick={() => {
                  if (confirm("Delete this task?")) remove.mutate(t.id);
                }}
                disabled={remove.isPending}
                className="rounded-md bg-red-600 px-2.5 py-1 text-xs font-medium text-white hover:bg-red-700 disabled:opacity-50"
              >
                Delete
              </button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}
