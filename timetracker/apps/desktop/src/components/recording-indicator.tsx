"use client";

import { useQuery } from "@tanstack/react-query";
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

  const recording = status.data === "working"; // screenshots are taken only while working
  const permissionMissing = capture.data === false;

  return (
    <>
      {permissionMissing && (
        <div className="fixed inset-x-0 top-0 z-[60] bg-red-600 py-1.5 text-center text-sm font-medium text-white">
          Screen-recording permission is not granted — screenshots cannot be captured.
          Enable it in your OS privacy settings.
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
