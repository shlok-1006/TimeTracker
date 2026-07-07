"use client";

import { useEffect, useState, type FormEvent } from "react";
import { useRouter } from "next/navigation";
import { login } from "@/lib/api";
import { useAuthStore } from "@/lib/auth-store";
import { cn } from "@/lib/utils";

export default function LoginPage() {
  const router = useRouter();
  const { user, token, hydrated, hydrate, setSession } = useAuthStore();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
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

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      const res = await login(email, password);
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
    } catch (err) {
      setError(err instanceof Error ? err.message : "Login failed.");
    } finally {
      setLoading(false);
    }
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
          <p className="text-sm text-muted-foreground">HR &amp; project manager sign in</p>
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
          Password
          <input
            type="password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="••••••••"
            className="rounded-md border border-input bg-background px-3 py-2"
          />
        </label>
        {error && <p className="text-sm text-red-600">{error}</p>}
        <button
          type="submit"
          disabled={loading}
          className={cn(
            "rounded-md bg-primary px-4 py-2 font-medium text-primary-foreground hover:opacity-90 disabled:opacity-50",
          )}
        >
          {loading ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </main>
  );
}
