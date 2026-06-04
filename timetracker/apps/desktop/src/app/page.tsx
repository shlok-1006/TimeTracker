"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { useSession } from "@/lib/session";

/** Entry point: restore session, then route to /dashboard or /login. */
export default function Home() {
  const router = useRouter();
  const { session, hydrated, hydrate } = useSession();

  useEffect(() => {
    if (!hydrated) hydrate();
  }, [hydrated, hydrate]);

  useEffect(() => {
    if (hydrated) router.replace(session ? "/dashboard" : "/login");
  }, [hydrated, session, router]);

  return (
    <main className="flex min-h-screen items-center justify-center text-slate-500">Loading…</main>
  );
}
