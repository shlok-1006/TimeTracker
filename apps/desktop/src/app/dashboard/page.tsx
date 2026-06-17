"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useSession } from "@/lib/session";
import { useEmployeeSession } from "@/components/use-employee-session";
import { Controls } from "@/components/controls";
import { Dashboard } from "@/components/dashboard";
import { DayReport } from "@/components/day-report";
import { MyTeams } from "@/components/my-teams";
import { MyTasks } from "@/components/my-tasks";
import { MyLeave } from "@/components/my-leave";
import { MyAttendance } from "@/components/my-attendance";
import { Tickets } from "@/components/tickets";
import { RecordingIndicator } from "@/components/recording-indicator";

type View = "dashboard" | "myday" | "work" | "leave" | "attendance";

const NAV: { key: View; label: string }[] = [
  { key: "dashboard", label: "Dashboard" },
  { key: "myday", label: "My Day" },
  { key: "work", label: "Tasks & Tickets" },
  { key: "leave", label: "Leave" },
  { key: "attendance", label: "Attendance" },
];

export default function DashboardPage() {
  const router = useRouter();
  const { session, ready } = useEmployeeSession();
  const clear = useSession((s) => s.clear);
  const [view, setView] = useState<View>("dashboard");

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
    <div className="flex h-screen">
      <RecordingIndicator />

      <aside className="flex h-screen w-56 shrink-0 flex-col border-r border-slate-200 bg-slate-50 dark:border-slate-800 dark:bg-slate-900">
        <div className="border-b border-slate-200 px-5 py-5 dark:border-slate-800">
          <h1 className="text-lg font-bold">TimeTracker</h1>
          <p className="text-xs text-slate-500">Employee</p>
        </div>

        <nav className="flex flex-1 flex-col gap-1 overflow-y-auto p-3">
          {NAV.map((item) => {
            const active = view === item.key;
            return (
              <button
                key={item.key}
                onClick={() => setView(item.key)}
                className={`rounded-md px-3 py-2 text-left text-sm font-medium transition ${
                  active
                    ? "bg-purple-100 text-purple-700 dark:bg-purple-950 dark:text-purple-300"
                    : "text-slate-700 hover:bg-slate-200 dark:text-slate-300 dark:hover:bg-slate-800"
                }`}
              >
                {item.label}
              </button>
            );
          })}
        </nav>

        <div className="border-t border-slate-200 p-3 dark:border-slate-800">
          <p className="truncate text-sm font-medium">{session.name}</p>
          <p className="mb-2 truncate text-xs text-slate-500">{session.email}</p>
          <button
            onClick={signOut}
            className="w-full rounded-md bg-slate-200 px-3 py-2 text-sm font-medium hover:bg-slate-300 dark:bg-slate-800 dark:hover:bg-slate-700"
          >
            Sign out
          </button>
        </div>
      </aside>

      <main className="min-w-0 flex-1 overflow-y-auto p-8">
        <div className="mx-auto flex max-w-4xl flex-col gap-6">
          {view === "dashboard" && (
            <>
              <header>
                <h2 className="text-2xl font-bold">Welcome, {session.name}</h2>
                <p className="text-slate-500">Your tracking and today&apos;s activity</p>
              </header>
              <Controls userId={session.id} />
              <MyTeams />
              <Dashboard userId={session.id} />
            </>
          )}
          {view === "myday" && <DayReport />}
          {view === "work" && (
            <>
              <MyTasks />
              <Tickets />
            </>
          )}
          {view === "leave" && <MyLeave />}
          {view === "attendance" && <MyAttendance />}
        </div>
      </main>
    </div>
  );
}
