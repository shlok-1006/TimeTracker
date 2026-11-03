"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoker, fmtHm } from "@/lib/tauri";

type SyncStatus = {
  pending_intervals: number;
  pending_tracked_seconds: number;
  oldest_pending_utc: string | null;
  pending_screenshots: number;
  offline: boolean;
  last_synced_utc: string | null;
  last_error: string | null;
};

/** A backlog this old means sync is stuck, even if the last attempt worked. */
const STALE_BACKLOG_MS = 10 * 60 * 1000;

function fmtTime(utc: string | null) {
  if (!utc) return "never";
  return new Date(utc).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/**
 * Tells the employee when their time is only on this computer — typically a
 * VPN that blocks the TimeTracker server. Nothing is lost (it is all saved
 * locally and syncs on its own); this just says so, and offers "Sync now" for
 * the moment the VPN is turned off. Hidden while everything is up to date.
 */
export function SyncBanner() {
  const queryClient = useQueryClient();
  const { data } = useQuery({
    queryKey: ["sync_status"],
    queryFn: async () => (await invoker())<SyncStatus>("sync_status"),
    refetchInterval: 15000,
  });
  const syncNow = useMutation({
    mutationFn: async () => (await invoker())<SyncStatus>("sync_now"),
    onSuccess: (status) => {
      queryClient.setQueryData(["sync_status"], status);
      queryClient.invalidateQueries({ queryKey: ["hours_summary"] });
    },
  });

  if (!data) return null;
  const waiting = data.pending_intervals > 0 || data.pending_screenshots > 0;
  const stale =
    data.oldest_pending_utc !== null &&
    Date.now() - new Date(data.oldest_pending_utc).getTime() > STALE_BACKLOG_MS;
  if (!waiting || !(data.offline || stale)) return null;

  const parts = [];
  if (data.pending_tracked_seconds > 0) parts.push(`${fmtHm(data.pending_tracked_seconds)} of tracked time`);
  if (data.pending_screenshots > 0)
    parts.push(`${data.pending_screenshots} screenshot${data.pending_screenshots === 1 ? "" : "s"}`);

  return (
    <div className="rounded-lg border border-amber-300 bg-amber-50 p-4 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
      <div className="flex items-start justify-between gap-4">
        <div>
          <p className="font-semibold">Can&apos;t reach the TimeTracker server</p>
          <p className="mt-1">
            {parts.length > 0 ? parts.join(" and ") : "Your latest activity"} {parts.length > 1 ? "are" : "is"} saved
            on this computer and will sync automatically. If you&apos;re on a VPN, your manager won&apos;t see
            it until you disconnect — nothing is lost.
          </p>
          <p className="mt-1 text-xs opacity-75">Last synced: {fmtTime(data.last_synced_utc)}</p>
          {data.last_error && (
            <details className="mt-1 text-xs opacity-75">
              <summary className="cursor-pointer">Details</summary>
              <p className="mt-1 break-all font-mono">{data.last_error}</p>
            </details>
          )}
        </div>
        <button
          onClick={() => syncNow.mutate()}
          disabled={syncNow.isPending}
          className="shrink-0 rounded-md bg-amber-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-amber-700 disabled:opacity-60"
        >
          {syncNow.isPending ? "Syncing…" : "Sync now"}
        </button>
      </div>
    </div>
  );
}
