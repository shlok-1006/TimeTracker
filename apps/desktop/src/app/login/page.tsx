"use client";

import { useEffect, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import { invoker, type EmployeeSession } from "@/lib/tauri";
import { useSession } from "@/lib/session";

export default function LoginPage() {
  const router = useRouter();
  const { session, hydrated, hydrate, setSession } = useSession();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [changing, setChanging] = useState(false);
  const [newPassword, setNewPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!hydrated) hydrate();
  }, [hydrated, hydrate]);

  useEffect(() => {
    if (hydrated && session) router.replace("/dashboard");
  }, [hydrated, session, router]);

  function toggleChanging() {
    setChanging((c) => !c);
    setError(null);
    setNewPassword("");
    setConfirmPassword("");
  }

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);

    if (changing) {
      // Mirror the server's rules for instant feedback (server re-validates).
      if (newPassword.length < 8) {
        setError("New password must be at least 8 characters.");
        return;
      }
      if (newPassword !== confirmPassword) {
        setError("New passwords do not match.");
        return;
      }
      if (newPassword === password) {
        setError("New password must be different from the current one.");
        return;
      }
    }

    setLoading(true);
    try {
      const invoke = await invoker();
      const result = changing
        ? await invoke<EmployeeSession>("change_password", {
            email,
            currentPassword: password,
            newPassword,
          })
        : await invoke<EmployeeSession>("login", { email, password });
      setSession(result);
      invoke("heartbeat_now").catch(() => {});
      router.replace("/dashboard");
    } catch (err) {
      setError(
        typeof err === "string"
          ? err
          : changing
            ? "Password change failed."
            : "Login failed.",
      );
    } finally {
      setLoading(false);
    }
  }

  const inputClass =
    "rounded-md border border-slate-300 px-3 py-2 dark:border-slate-700 dark:bg-slate-900";

  return (
    <main className="mx-auto flex min-h-screen max-w-sm flex-col justify-center gap-6 p-8">
      <header>
        <h1 className="text-3xl font-bold">TimeTracker</h1>
        <p className="text-slate-500">
          {changing ? "Change your password" : "Employee sign in"}
        </p>
      </header>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        <label className="flex flex-col gap-1 text-sm">
          Email
          <input
            type="email"
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className={inputClass}
            placeholder="employee@timetracker.local"
          />
        </label>
        <label className="flex flex-col gap-1 text-sm">
          {changing ? "Current password" : "Password"}
          <input
            type="password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            className={inputClass}
            placeholder="••••••••"
          />
        </label>

        {changing && (
          <>
            <label className="flex flex-col gap-1 text-sm">
              New password
              <input
                type="password"
                required
                value={newPassword}
                onChange={(e) => setNewPassword(e.target.value)}
                className={inputClass}
                placeholder="At least 8 characters"
              />
            </label>
            <label className="flex flex-col gap-1 text-sm">
              Confirm new password
              <input
                type="password"
                required
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                className={inputClass}
                placeholder="••••••••"
              />
            </label>
          </>
        )}

        {error && <p className="text-sm text-red-600">{error}</p>}

        <button
          type="submit"
          disabled={loading}
          className="rounded-md bg-purple-600 px-4 py-2 font-medium text-white hover:bg-purple-700 disabled:opacity-50"
        >
          {loading
            ? changing
              ? "Updating…"
              : "Signing in…"
            : changing
              ? "Change password & sign in"
              : "Sign in"}
        </button>

        <button
          type="button"
          onClick={toggleChanging}
          className="text-sm text-purple-600 hover:underline"
        >
          {changing ? "Back to sign in" : "Change password"}
        </button>
      </form>
    </main>
  );
}
