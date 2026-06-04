"use client";

import { useState } from "react";
import type { AdminShot } from "@/lib/api";

/** Thumbnail grid + click-to-zoom modal. Images that fail to load are hidden
 *  (e.g. when the object store is offline). */
export function ScreenshotGallery({ shots }: { shots: AdminShot[] }) {
  const [zoom, setZoom] = useState<string | null>(null);

  if (shots.length === 0) {
    return <p className="text-sm text-muted-foreground">No screenshots.</p>;
  }

  return (
    <>
      <div className="grid grid-cols-3 gap-3 sm:grid-cols-4">
        {shots.map((s) => (
          <button
            key={s.id}
            onClick={() => setZoom(s.url)}
            className="overflow-hidden rounded-md border"
            title={new Date(s.taken_at).toLocaleString()}
          >
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src={s.url}
              alt="screenshot"
              className="h-24 w-full object-cover"
              onError={(e) => ((e.currentTarget as HTMLImageElement).style.display = "none")}
            />
          </button>
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
