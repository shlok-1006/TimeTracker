"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

type Phase =
  | { kind: "ready" }
  | { kind: "downloading"; percent: number | null }
  | { kind: "installing" }
  | { kind: "failed"; reason: string };

type Progress = { downloaded: number; total: number | null };

/**
 * Prompts the user to update when a newer release is available. Checks on mount
 * and every few hours (plus on window focus). A check that fails — offline or
 * GitHub rate-limited — resolves to `null` and shows nothing, so a missed check
 * never turns into a false prompt.
 *
 * "Update now" downloads and installs the new version, then the app restarts
 * into it by itself (a running timer carries on). If that can't happen — no
 * signed update published for this computer, a failed download — the banner
 * says so and offers the downloads page instead.
 */
export function UpdateBanner() {
  const [phase, setPhase] = useState<Phase>({ kind: "ready" });
  const { data: latest } = useQuery({
    queryKey: ["check_for_update"],
    queryFn: async () => (await invoker())<string | null>("check_for_update"),
    refetchInterval: 6 * 60 * 60 * 1000, // every 6 hours
    refetchOnWindowFocus: true,
    staleTime: 60 * 60 * 1000,
    retry: false,
  });

  async function updateNow() {
    setPhase({ kind: "downloading", percent: null });
    const { listen } = await import("@tauri-apps/api/event");
    const stopProgress = await listen<Progress>("update-progress", ({ payload }) => {
      const percent = payload.total
        ? Math.min(100, Math.round((payload.downloaded / payload.total) * 100))
        : null;
      setPhase({ kind: "downloading", percent });
    });
    const stopInstalling = await listen("update-installing", () => setPhase({ kind: "installing" }));
    try {
      // Resolves only on failure: on success the app restarts into the new version.
      await (await invoker())("install_update");
    } catch (e) {
      setPhase({ kind: "failed", reason: String(e) });
    } finally {
      stopProgress();
      stopInstalling();
    }
  }

  if (!latest) return null;

  const button =
    "rounded bg-slate-900/10 px-2 py-0.5 text-xs font-semibold hover:bg-slate-900/20 disabled:opacity-60";

  return (
    <div className="fixed inset-x-0 top-0 z-[70] flex items-center justify-center gap-3 bg-amber-500 px-4 py-2 text-center text-sm font-medium text-slate-900">
      {phase.kind === "ready" && (
        <>
          <span>
            A newer version (v{latest}) is available — please update to keep your tracking accurate.
          </span>
          <button onClick={() => void updateNow()} className={button}>
            Update now
          </button>
        </>
      )}
      {phase.kind === "downloading" && (
        <span>
          Downloading v{latest}
          {phase.percent !== null ? ` — ${phase.percent}%` : "…"}
        </span>
      )}
      {phase.kind === "installing" && (
        <span>Installing v{latest} — TimeTracker will restart in a moment.</span>
      )}
      {phase.kind === "failed" && (
        <>
          <span title={phase.reason}>Couldn&apos;t update automatically.</span>
          <button onClick={() => void updateNow()} className={button}>
            Try again
          </button>
          <button
            onClick={() => invoker().then((i) => i("open_downloads_page"))}
            className={button}
          >
            Download manually
          </button>
        </>
      )}
    </div>
  );
}
