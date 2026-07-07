"use client";

import { useEmployeeSession } from "@/components/use-employee-session";

export default function PerformancePage() {
  const { ready } = useEmployeeSession();

  if (!ready) {
    return (
      <main className="flex min-h-screen items-center justify-center text-muted-foreground">
        Loading…
      </main>
    );
  }

  return (
    <main className="p-8">
      <h1 className="text-2xl font-bold tracking-tight">My Performance</h1>
      <p className="mt-1 text-sm text-muted-foreground">
        Your personal performance analytics.
      </p>

      <div className="mt-8 flex flex-col items-center justify-center rounded-lg border border-dashed p-16 text-center">
        <p className="text-lg font-medium">Coming soon</p>
        <p className="mt-1 max-w-md text-sm text-muted-foreground">
          The performance analyzer is on its way. Soon you&apos;ll see productivity
          trends, focus insights, and activity breakdowns here.
        </p>
      </div>
    </main>
  );
}
