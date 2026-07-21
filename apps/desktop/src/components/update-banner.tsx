"use client";

import { useQuery } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

/**
 * Prompts the user to update when a newer release is available. Checks on mount
 * and every few hours (plus on window focus). A check that fails — offline or
 * GitHub rate-limited — resolves to `null` and shows nothing, so a missed check
 * never turns into a false prompt.
 */
export function UpdateBanner() {
  const { data: latest } = useQuery({
    queryKey: ["check_for_update"],
    queryFn: async () => (await invoker())<string | null>("check_for_update"),
    refetchInterval: 6 * 60 * 60 * 1000, // every 6 hours
    refetchOnWindowFocus: true,
    staleTime: 60 * 60 * 1000,
    retry: false,
  });

  if (!latest) return null;

  return (
    <div className="fixed inset-x-0 top-0 z-[70] flex items-center justify-center gap-3 bg-amber-500 px-4 py-2 text-center text-sm font-medium text-slate-900">
      <span>
        A newer version (v{latest}) is available — please update to keep your tracking accurate.
      </span>
      <button
        onClick={() => invoker().then((i) => i("open_downloads_page"))}
        className="rounded bg-slate-900/10 px-2 py-0.5 text-xs font-semibold hover:bg-slate-900/20"
      >
        Update now
      </button>
    </div>
  );
}
