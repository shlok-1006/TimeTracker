"use client";

import type { UserActivity } from "@/lib/api";
import { fmtHms } from "@/lib/format";

/** Aggregate 10-minute blocks into per-hour activity percentages (UTC). */
function hourlyPct(blocks: UserActivity["blocks"]): { hour: number; pct: number | null }[] {
  const active = new Array<number>(24).fill(0);
  const total = new Array<number>(24).fill(0);
  for (const b of blocks) {
    const h = new Date(b.block_start).getUTCHours();
    active[h] += b.active_seconds;
    total[h] += b.total_seconds;
  }
  return Array.from({ length: 24 }, (_, hour) => ({
    hour,
    pct: total[hour] > 0 ? (active[hour] / total[hour]) * 100 : null,
  }));
}

/** Per-employee activity for one day: overall %, hourly bars, top apps.
 *  App names only — window titles are never collected. */
export function ActivityCard({ activity }: { activity: UserActivity }) {
  const hours = hourlyPct(activity.blocks);
  const hasData = activity.apps.length > 0 || activity.blocks.length > 0;
  const maxApp = activity.apps[0]?.seconds || 1;

  if (!hasData) {
    return (
      <p className="text-sm text-muted-foreground">
        No activity data for this day — it appears once the employee tracks on
        a desktop app version with activity recording (v0.1.7+).
      </p>
    );
  }

  return (
    <div className="flex flex-col gap-5">
      <div className="flex items-baseline gap-3">
        <span className="text-3xl font-semibold tabular-nums">
          {activity.activity_pct != null ? `${Math.round(activity.activity_pct)}%` : "—"}
        </span>
        <span className="text-sm text-muted-foreground">
          overall input activity while tracking
        </span>
      </div>

      {/* Hourly bars (UTC): height = activity % for that hour. */}
      <div>
        <div className="flex h-24 items-end gap-1">
          {hours.map((h) => (
            <div
              key={h.hour}
              className="flex-1 rounded-t bg-secondary"
              title={h.pct != null ? `${String(h.hour).padStart(2, "0")}:00 — ${Math.round(h.pct)}%` : undefined}
            >
              {h.pct != null && (
                <div
                  className="w-full rounded-t bg-primary"
                  style={{ height: `${Math.max(h.pct, 4)}%` }}
                />
              )}
            </div>
          ))}
        </div>
        <div className="mt-1 flex justify-between text-[10px] text-muted-foreground">
          <span>00:00</span>
          <span>06:00</span>
          <span>12:00</span>
          <span>18:00</span>
          <span>24:00 (UTC)</span>
        </div>
      </div>

      {/* Top applications by foreground time. */}
      {activity.apps.length > 0 && (
        <div className="flex flex-col gap-2">
          <h3 className="text-sm font-medium">Applications</h3>
          {activity.apps.slice(0, 8).map((a) => (
            <div key={a.app_name} className="flex items-center gap-3 text-sm">
              <span className="w-44 truncate" title={a.app_name}>
                {a.app_name}
              </span>
              <div className="h-2.5 flex-1 overflow-hidden rounded-full bg-secondary">
                <div
                  className="h-full rounded-full bg-primary"
                  style={{ width: `${(a.seconds / maxApp) * 100}%` }}
                />
              </div>
              <span className="w-20 text-right tabular-nums text-muted-foreground">
                {fmtHms(a.seconds)}
              </span>
            </div>
          ))}
          <p className="mt-1 text-xs text-muted-foreground">
            App names only — window titles and keystrokes are never recorded.
          </p>
        </div>
      )}
    </div>
  );
}
