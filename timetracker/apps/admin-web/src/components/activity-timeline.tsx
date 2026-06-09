"use client";

import type { DayTimeline, TimelineSegment } from "@/lib/api";

const COLORS: Record<TimelineSegment["kind"], string> = {
  active: "#22c55e", // green
  idle: "#f59e0b", // amber
  meeting: "#a855f7", // purple
  break: "#3b82f6", // blue
};
const LABELS: Record<TimelineSegment["kind"], string> = {
  active: "Active",
  idle: "Idle",
  meeting: "Meeting",
  break: "Break",
};

function hhmm(ms: number) {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** A proportional, colored activity bar across the selected day. */
export function ActivityTimeline({ data }: { data: DayTimeline }) {
  const from = new Date(data.from).getTime();
  const to = new Date(data.to).getTime();
  const span = Math.max(1, to - from);

  // Hour ticks every 3 hours across the window.
  const ticks = Array.from({ length: 9 }, (_, i) => from + i * 3 * 3600 * 1000);

  // Which kinds are actually present (for a relevant legend).
  const present = new Set(data.segments.map((s) => s.kind));

  return (
    <div>
      <div className="relative h-9 w-full overflow-hidden rounded bg-slate-200 dark:bg-slate-700">
        {data.segments.map((s, i) => {
          const a = Math.max(0, (new Date(s.start_utc).getTime() - from) / span);
          const b = Math.min(1, (new Date(s.end_utc).getTime() - from) / span);
          if (b <= a) return null;
          return (
            <div
              key={i}
              className="absolute top-0 h-full"
              style={{ left: `${a * 100}%`, width: `${Math.max(0.2, (b - a) * 100)}%`, background: COLORS[s.kind] }}
              title={`${LABELS[s.kind]} · ${hhmm(new Date(s.start_utc).getTime())}–${hhmm(new Date(s.end_utc).getTime())}`}
            />
          );
        })}
      </div>

      <div className="relative mt-1 h-4 w-full text-[10px] text-muted-foreground">
        {ticks.map((t, i) => (
          <span key={i} className="absolute -translate-x-1/2" style={{ left: `${(i / 8) * 100}%` }}>
            {hhmm(t)}
          </span>
        ))}
      </div>

      <div className="mt-3 flex flex-wrap gap-4 text-sm">
        {(["active", "idle", "meeting", "break"] as const)
          .filter((k) => present.has(k) || k === "active" || k === "idle")
          .map((k) => (
            <span key={k} className="inline-flex items-center gap-2">
              <span className="h-2.5 w-2.5 rounded-sm" style={{ background: COLORS[k] }} />
              {LABELS[k]}
            </span>
          ))}
        <span className="inline-flex items-center gap-2 text-muted-foreground">
          <span className="h-2.5 w-2.5 rounded-sm bg-slate-200 dark:bg-slate-700" /> Untracked
        </span>
      </div>
    </div>
  );
}
