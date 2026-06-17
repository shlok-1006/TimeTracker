"use client";

import { useState } from "react";
import type { DayShot } from "@/lib/api";

const VERDICT_BADGE: Record<string, string> = {
  aligned: "bg-green-100 text-green-800",
  partially_aligned: "bg-lime-100 text-lime-800",
  not_aligned: "bg-red-100 text-red-800",
  inconclusive: "bg-slate-100 text-slate-700",
};

function timeLabel(iso: string) {
  return new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** Day-based screenshot grid: each tile shows its capture time and an analysis
 *  badge (verdict, or "Meeting · not analysed"). Click to zoom. */
export function DayGallery({ shots }: { shots: DayShot[] }) {
  const [zoom, setZoom] = useState<string | null>(null);

  if (shots.length === 0) {
    return <p className="text-sm text-muted-foreground">No screenshots for this day.</p>;
  }

  return (
    <>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        {shots.map((s) => (
          <div key={s.screenshot.id} className="overflow-hidden rounded-md border">
            <button
              onClick={() => setZoom(s.presigned_url)}
              className="block w-full"
              title={new Date(s.screenshot.taken_at).toLocaleString()}
            >
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img
                src={s.presigned_url}
                alt="screenshot"
                className="h-28 w-full object-cover"
                onError={(e) => ((e.currentTarget as HTMLImageElement).style.display = "none")}
              />
            </button>
            <div className="flex items-center justify-between gap-1 px-2 py-1.5">
              <span className="text-[10px] tabular-nums text-muted-foreground">
                {timeLabel(s.screenshot.taken_at)}
              </span>
              {s.meeting_flag ? (
                <span className="rounded-full bg-purple-100 px-2 py-0.5 text-[10px] font-medium text-purple-800">
                  Meeting · not analysed
                </span>
              ) : s.verdict ? (
                <span
                  className={`rounded-full px-2 py-0.5 text-[10px] font-medium ${
                    VERDICT_BADGE[s.verdict] ?? "bg-slate-100 text-slate-700"
                  }`}
                >
                  {s.verdict.replace(/_/g, " ")}
                </span>
              ) : (
                <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[10px] text-slate-500">
                  not analysed
                </span>
              )}
            </div>
          </div>
        ))}
      </div>

      {zoom && (
        <div
          className="fixed inset-0 z-[60] flex items-center justify-center bg-black/80 p-6"
          onClick={() => setZoom(null)}
        >
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={zoom} alt="screenshot" className="max-h-[85vh] w-auto" />
        </div>
      )}
    </>
  );
}
