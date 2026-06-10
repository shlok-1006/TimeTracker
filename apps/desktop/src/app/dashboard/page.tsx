"use client";

import { useRouter } from "next/navigation";
import { useSession } from "@/lib/session";
import { useEmployeeSession } from "@/components/use-employee-session";
import { Controls } from "@/components/controls";
import { Dashboard } from "@/components/dashboard";
import { Tickets } from "@/components/tickets";
import { RecordingIndicator } from "@/components/recording-indicator";

export default function DashboardPage() {
  const router = useRouter();
  const { session, ready } = useEmployeeSession();
  const clear = useSession((s) => s.clear);

  async function signOut() {
    await clear();
    router.replace("/login");
  }

  if (!ready || !session) {
    return (
      <main className="flex min-h-screen items-center justify-center text-slate-500">Loading…</main>
    );
  }

  return (
    <main className="mx-auto flex max-w-4xl flex-col gap-6 p-8">
      <RecordingIndicator />
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Welcome, {session.name}</h1>
          <p className="text-slate-500">Employee dashboard</p>
        </div>
        <button
          onClick={signOut}
          className="rounded-md bg-slate-200 px-3 py-1.5 text-sm font-medium hover:bg-slate-300 dark:bg-slate-800 dark:hover:bg-slate-700"
        >
          Sign out
        </button>
      </header>
      <Controls userId={session.id} />
      <Tickets />
      <Dashboard userId={session.id} />
    </main>
  );
}
