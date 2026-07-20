"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { useSession } from "@/lib/session";
import { invoker } from "@/lib/tauri";

/** How often to re-verify the session while signed in. */
const SESSION_CHECK_MS = 60_000;

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

  // Keep the session honest. If the token quietly dies mid-session (revoked or
  // expired), the app must not keep "tracking" while the server sees nothing —
  // `session_alive` returns false only when we're DEFINITIVELY signed out
  // (transient network errors throw and are ignored), so we boot to login with
  // a clear "session expired" note and the user re-signs in.
  useEffect(() => {
    if (!session) return;
    let cancelled = false;
    const check = async () => {
      try {
        const invoke = await invoker();
        const alive = await invoke<boolean>("session_alive");
        if (!cancelled && alive === false) {
          await useSession.getState().markExpired();
        }
      } catch {
        /* transient (offline / server hiccup / not in Tauri) — keep the session */
      }
    };
    const id = setInterval(check, SESSION_CHECK_MS);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, [session]);

  return { session, ready: hydrated && !!session };
}
