"use client";

import { useEffect, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import { changePassword, login, type LoginResponse } from "@/lib/api";
import { useAuthStore } from "@/lib/auth-store";
import { cn } from "@/lib/utils";

export default function LoginPage() {
  const router = useRouter();
  const { user, token, hydrated, hydrate, setSession } = useAuthStore();
  const [changing, setChanging] = useState(false);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    hydrate();
  }, [hydrate]);

  // Already signed in → HR lands on Manage users, PMs on the team dashboard.
  useEffect(() => {
    if (hydrated && user && token) {
      router.replace(user.role === "hr" ? "/manage" : "/dashboard");
    }
  }, [hydrated, user, token, router]);

  /** Both sign-in and change-password return a full session. */
  function applySession(res: LoginResponse) {
    if (res.user.role === "employee") {
      setError("This dashboard is for HR and project managers only.");
      return;
    }
    setSession(res.access_token, res.refresh_token, {
      id: res.user.id,
      name: res.user.name,
      email: res.user.email,
      role: res.user.role,
      team: res.user.team,
    });
    router.replace(res.user.role === "hr" ? "/manage" : "/dashboard");
  }

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      const res = changing
        ? await changePassword(email, password, newPassword)
        : await login(email, password);
      applySession(res);
    } catch (err) {
      setError(err instanceof Error ? err.message : changing ? "Password change failed." : "Login failed.");
    } finally {
      setLoading(false);
    }
  }

  function toggleMode() {
    setChanging((c) => !c);
    setNewPassword("");
    setError(null);
  }

  return (
    <main className="flex min-h-screen items-center justify-center p-6">
      <form
        onSubmit={onSubmit}
        className="flex w-full max-w-sm flex-col gap-5 rounded-xl border bg-card p-8 text-card-foreground"
      >
        <header className="flex flex-col gap-2">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src="/ruh-logo.svg" alt="RUH" className="h-8 w-auto self-start dark:brightness-0 dark:invert" />
          <p className="text-sm text-muted-foreground">
            {changing ? "Change your password" : "HR & project manager sign in"}
          </p>
        </header>
        <label className="flex flex-col gap-1 text-sm">
          Email
          <input
            type="email"
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="hr@timetracker.local"
            className="rounded-md border border-input bg-background px-3 py-2"
          />
        </label>
        <label className="flex flex-col gap-1 text-sm">
          {changing ? "Current password" : "Password"}
          <input
            type="password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="••••••••"
            className="rounded-md border border-input bg-background px-3 py-2"
          />
        </label>
        {changing && (
          <label className="flex flex-col gap-1 text-sm">
            New password
            <input
              type="password"
              required
              minLength={8}
              value={newPassword}
              onChange={(e) => setNewPassword(e.target.value)}
              placeholder="At least 8 characters"
              className="rounded-md border border-input bg-background px-3 py-2"
            />
          </label>
        )}
        {error && <p className="text-sm text-red-600">{error}</p>}
        <button
          type="submit"
          disabled={loading}
          className={cn(
            "rounded-md bg-primary px-4 py-2 font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50",
          )}
        >
          {loading
            ? changing
              ? "Updating…"
              : "Signing in…"
            : changing
              ? "Set new password & sign in"
              : "Sign in"}
        </button>
        <button
          type="button"
          onClick={toggleMode}
          className="self-center text-xs text-muted-foreground underline hover:text-foreground"
        >
          {changing ? "← Back to sign in" : "Reset / change password"}
        </button>
      </form>
    </main>
  );
}
