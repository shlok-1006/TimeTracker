"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { useSession } from "@/lib/session";

/** Guard hook: restore the session and redirect to /login if not signed in. */
export function useEmployeeSession() {
  const { session, hydrated, hydrate } = useSession();
  const router = useRouter();

  useEffect(() => {
    if (!hydrated) hydrate();
  }, [hydrated, hydrate]);

  useEffect(() => {
    if (hydrated && !session) router.replace("/login");
  }, [hydrated, session, router]);

  return { session, ready: hydrated && !!session };
}
