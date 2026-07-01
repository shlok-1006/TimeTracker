"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { useAuthStore } from "@/lib/auth-store";

/** Entry point: send to /dashboard if signed in, else /login. */
export default function Home() {
  const router = useRouter();
  const { user, token, hydrated, hydrate } = useAuthStore();

  useEffect(() => {
    hydrate();
  }, [hydrate]);

  useEffect(() => {
    if (!hydrated) return;
    router.replace(!user || !token ? "/login" : "/dashboard");
  }, [hydrated, user, token, router]);

  return (
    <main className="flex min-h-screen items-center justify-center text-muted-foreground">
      Loading…
    </main>
  );
}
