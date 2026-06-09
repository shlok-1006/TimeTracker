"use client";

import { useEffect, useState, type FormEvent } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { listUsers, createUser, deleteUser, resetPassword, type ManagedUser } from "@/lib/api";
import { useAdminSession } from "@/components/use-admin-session";

const ROLE_LABEL: Record<string, string> = {
  employee: "Employee",
  project_manager: "Project manager",
  hr: "HR",
};

export default function ManageUsersPage() {
  const router = useRouter();
  const { user, ready } = useAdminSession();
  const qc = useQueryClient();

  // HR-only page (the API enforces it too).
  useEffect(() => {
    if (ready && user && user.role !== "hr") router.replace("/dashboard");
  }, [ready, user, router]);

  const users = useQuery({
    queryKey: ["users"],
    queryFn: listUsers,
    enabled: ready && user?.role === "hr",
  });

  const managers = (users.data ?? []).filter((u) => u.role === "project_manager");

  const [form, setForm] = useState({
    name: "",
    email: "",
    password: "",
    role: "employee" as ManagedUser["role"],
    manager_id: "",
  });
  const [formError, setFormError] = useState<string | null>(null);

  const create = useMutation({
    mutationFn: () =>
      createUser({
        name: form.name,
        email: form.email,
        password: form.password,
        role: form.role,
        manager_id: form.manager_id || null,
      }),
    onSuccess: () => {
      setForm({ name: "", email: "", password: "", role: "employee", manager_id: "" });
      setFormError(null);
      qc.invalidateQueries({ queryKey: ["users"] });
    },
    onError: (e) => setFormError(e instanceof Error ? e.message : "Failed to create user."),
  });

  const remove = useMutation({
    mutationFn: (id: string) => deleteUser(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["users"] }),
  });

  const [issued, setIssued] = useState<{ name: string; password: string } | null>(null);
  const reset = useMutation({
    mutationFn: async (u: ManagedUser) => ({ name: u.name, password: await resetPassword(u.id) }),
    onSuccess: (r) => setIssued(r),
  });

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    setFormError(null);
    create.mutate();
  }

  if (!ready || user?.role !== "hr") {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="container mx-auto flex max-w-4xl flex-col gap-6 py-12">
      <header className="flex items-center justify-between">
        <h1 className="text-3xl font-bold tracking-tight">Manage users</h1>
        <Link
          href="/dashboard"
          className="rounded-md bg-secondary px-4 py-2 text-sm font-medium hover:opacity-90"
        >
          ← Back to team
        </Link>
      </header>

      {/* New-password banner (shown once after a reset) */}
      {issued && (
        <div className="rounded-lg border border-green-300 bg-green-50 p-4 text-green-900">
          <div className="flex items-start justify-between gap-4">
            <p className="text-sm">
              New password for <strong>{issued.name}</strong>:{" "}
              <code className="rounded bg-white px-2 py-0.5 font-mono">{issued.password}</code>
              <br />
              Share it securely — it won&apos;t be shown again (it&apos;s stored only as a hash).
            </p>
            <button onClick={() => setIssued(null)} className="text-sm underline">
              Dismiss
            </button>
          </div>
        </div>
      )}

      {/* Add user */}
      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-4 text-lg font-semibold">Add a user</h2>
        <form onSubmit={onSubmit} className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <input
            required placeholder="Full name" value={form.name}
            onChange={(e) => setForm({ ...form, name: e.target.value })}
            className="rounded-md border border-input bg-background px-3 py-2"
          />
          <input
            required type="email" placeholder="email@company.com" value={form.email}
            onChange={(e) => setForm({ ...form, email: e.target.value })}
            className="rounded-md border border-input bg-background px-3 py-2"
          />
          <input
            required type="password" placeholder="Temp password (min 8)" value={form.password}
            onChange={(e) => setForm({ ...form, password: e.target.value })}
            className="rounded-md border border-input bg-background px-3 py-2"
          />
          <select
            value={form.role}
            onChange={(e) => setForm({ ...form, role: e.target.value as ManagedUser["role"] })}
            className="rounded-md border border-input bg-background px-3 py-2"
          >
            <option value="employee">Employee</option>
            <option value="project_manager">Project manager</option>
            <option value="hr">HR</option>
          </select>
          {form.role === "employee" && (
            <select
              value={form.manager_id}
              onChange={(e) => setForm({ ...form, manager_id: e.target.value })}
              className="rounded-md border border-input bg-background px-3 py-2"
            >
              <option value="">No manager</option>
              {managers.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name} ({m.email})
                </option>
              ))}
            </select>
          )}
          <button
            type="submit"
            disabled={create.isPending}
            className="rounded-md bg-primary px-4 py-2 font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50 sm:col-span-2"
          >
            {create.isPending ? "Adding…" : "Add user"}
          </button>
        </form>
        {formError && <p className="mt-3 text-sm text-red-600">{formError}</p>}
      </section>

      {/* Users list */}
      <section className="rounded-lg border bg-card p-6 text-card-foreground">
        <h2 className="mb-4 text-lg font-semibold">Users</h2>
        {users.isLoading && <p className="text-muted-foreground">Loading…</p>}
        {users.error && <p className="text-red-600">{(users.error as Error).message}</p>}
        {users.data && (
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b text-left text-muted-foreground">
                <th className="py-2 font-medium">Name</th>
                <th className="py-2 font-medium">Role</th>
                <th className="py-2 font-medium" />
              </tr>
            </thead>
            <tbody>
              {users.data.map((u) => (
                <tr key={u.id} className="border-b last:border-0">
                  <td className="py-2">
                    <div className="font-medium">{u.name}</div>
                    <div className="text-xs text-muted-foreground">{u.email}</div>
                  </td>
                  <td className="py-2">{ROLE_LABEL[u.role] ?? u.role}</td>
                  <td className="py-2">
                    <div className="flex justify-end gap-2">
                      <button
                        onClick={() => reset.mutate(u)}
                        disabled={reset.isPending}
                        className="rounded-md bg-secondary px-3 py-1.5 text-xs font-medium hover:opacity-90 disabled:opacity-50"
                      >
                        Reset password
                      </button>
                      {u.id !== user?.id && (
                        <button
                          onClick={() => {
                            if (confirm(`Delete ${u.name}? This removes all their data.`)) {
                              remove.mutate(u.id);
                            }
                          }}
                          disabled={remove.isPending}
                          className="rounded-md bg-red-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-red-700 disabled:opacity-50"
                        >
                          Delete
                        </button>
                      )}
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </main>
  );
}
