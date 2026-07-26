"use client";

import { useCallback, useEffect } from "react";

export type LightboxItem = { url: string; caption?: string };

/** Full-screen image viewer with previous/next navigation (arrows, keyboard
 *  ←/→, Esc to close). Navigation wraps around the list. Controlled: the parent
 *  owns the open index so it can be opened from a thumbnail grid. */
export function Lightbox({
  items,
  index,
  onIndexChange,
  onClose,
}: {
  items: LightboxItem[];
  index: number;
  onIndexChange: (i: number) => void;
  onClose: () => void;
}) {
  const count = items.length;
  const go = useCallback(
    (delta: number) => onIndexChange((index + delta + count) % count),
    [index, count, onIndexChange],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      else if (e.key === "ArrowLeft") go(-1);
      else if (e.key === "ArrowRight") go(1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [go, onClose]);

  const item = items[index];
  if (!item) return null;

  return (
    <div
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/85 p-6"
      onClick={onClose}
    >
      <button
        onClick={onClose}
        aria-label="Close"
        className="absolute right-4 top-4 flex h-9 w-9 items-center justify-center rounded-full bg-white/10 text-lg text-white hover:bg-white/20"
      >
        ✕
      </button>

      {count > 1 && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            go(-1);
          }}
          aria-label="Previous screenshot"
          className="absolute left-3 flex h-11 w-11 items-center justify-center rounded-full bg-white/10 text-2xl text-white hover:bg-white/20 sm:left-6"
        >
          ‹
        </button>
      )}

      <div
        className="flex max-h-full flex-col items-center gap-3"
        onClick={(e) => e.stopPropagation()}
      >
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src={item.url}
          alt="screenshot"
          className="max-h-[82vh] w-auto rounded-md"
          onError={(e) => ((e.currentTarget as HTMLImageElement).style.opacity = "0.15")}
        />
        <div className="text-xs text-white/80">
          {item.caption ? `${item.caption} · ` : ""}
          {index + 1} / {count}
        </div>
      </div>

      {count > 1 && (
        <button
          onClick={(e) => {
            e.stopPropagation();
            go(1);
          }}
          aria-label="Next screenshot"
          className="absolute right-3 flex h-11 w-11 items-center justify-center rounded-full bg-white/10 text-2xl text-white hover:bg-white/20 sm:right-6"
        >
          ›
        </button>
      )}
    </div>
  );
}
