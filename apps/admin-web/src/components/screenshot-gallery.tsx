"use client";

import { useState } from "react";
import type { AdminShot } from "@/lib/api";
import { Lightbox } from "@/components/lightbox";

/** Thumbnail grid + click-to-open viewer with prev/next navigation. Images that
 *  fail to load are hidden (e.g. when the object store is offline). */
export function ScreenshotGallery({ shots }: { shots: AdminShot[] }) {
  const [openIndex, setOpenIndex] = useState<number | null>(null);

  if (shots.length === 0) {
    return <p className="text-sm text-muted-foreground">No screenshots.</p>;
  }

  const items = shots.map((s) => ({
    url: s.url,
    caption: new Date(s.taken_at).toLocaleString(),
  }));

  return (
    <>
      <div className="grid grid-cols-3 gap-3 sm:grid-cols-4">
        {shots.map((s, i) => (
          <button
            key={s.id}
            onClick={() => setOpenIndex(i)}
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

      {openIndex !== null && (
        <Lightbox
          items={items}
          index={openIndex}
          onIndexChange={setOpenIndex}
          onClose={() => setOpenIndex(null)}
        />
      )}
    </>
  );
}
