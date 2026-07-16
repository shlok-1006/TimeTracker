"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { invoker } from "@/lib/tauri";

/**
 * Transparency (ethics): an always-visible indicator while screen recording is
 * active, plus a warning if the OS hasn't granted screen-recording permission.
 */
export function RecordingIndicator() {
  const status = useQuery({
    queryKey: ["current_status"],
    queryFn: async () => (await invoker())<string>("current_status"),
    refetchInterval: 5000,
  });
  const capture = useQuery({
    queryKey: ["check_capture"],
    queryFn: async () => (await invoker())<boolean>("check_capture"),
    refetchInterval: 60000,
  });

  const qc = useQueryClient();
  const requestPermission = useMutation({
    mutationFn: async () => (await invoker())<boolean>("request_capture_permission"),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["check_capture"] }),
  });
  // macOS caches the permission decision per process, so enabling it in
  // Settings only takes effect after the app restarts. This applies the grant
  // without the user having to manually quit and reopen.
  const relaunch = useMutation({
    mutationFn: async () => (await invoker())<void>("relaunch_app"),
  });

  const recording = status.data === "working"; // screenshots are taken only while working
  const permissionMissing = capture.data === false;

  return (
    <>
      {permissionMissing && (
        <div className="fixed inset-x-0 top-0 z-[60] bg-red-600 px-4 py-2 text-center text-sm font-medium text-white">
          Screen recording is not permitted — screenshots will miss your work.{" "}
          <span className="font-normal">
            Step 1: click <b>Open settings</b> and enable <b>TimeTracker</b> under Screen
            Recording. Step 2: click <b>Restart app</b> to apply it.
          </span>{" "}
          <button
            onClick={() => requestPermission.mutate()}
            className="ml-2 rounded bg-white/20 px-2 py-0.5 text-xs font-semibold hover:bg-white/30"
          >
            Open settings
          </button>
          <button
            onClick={() => relaunch.mutate()}
            className="ml-2 rounded bg-white/20 px-2 py-0.5 text-xs font-semibold hover:bg-white/30"
          >
            Restart app
          </button>
        </div>
      )}
      {recording && (
        <div className="fixed bottom-4 right-4 z-50 inline-flex items-center gap-2 rounded-full bg-red-600/90 px-3 py-1.5 text-sm font-medium text-white shadow-lg">
          <span className="h-2.5 w-2.5 animate-pulse rounded-full bg-white" />
          Screen recording active
        </div>
      )}
    </>
  );
}
