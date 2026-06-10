"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { useAuthStore } from "@/lib/auth-store";

/**
 * Guard hook for admin pages: hydrates the auth store and redirects to /login
 * if there is no session. `ready` is true only once hydrated and authenticated.
 */
export function useAdminSession() {
  const { user, token, hydrated, hydrate } = useAuthStore();
  const router = useRouter();

  useEffect(() => {
    hydrate();
  }, [hydrate]);

  useEffect(() => {
    if (hydrated && (!user || !token)) {
      router.replace("/login");
    }
  }, [hydrated, user, token, router]);

  return { user, token, ready: hydrated && !!user && !!token };
}
