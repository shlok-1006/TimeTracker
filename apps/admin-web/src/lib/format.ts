/** Format seconds as "Hh Mm". */
export function fmtHms(total: number): string {
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  return `${h}h ${m}m`;
}

/** Human "x ago" from an ISO timestamp (or "never"). */
export function timeAgo(iso: string | null): string {
  if (!iso) return "never";
  const secs = Math.max(0, Math.floor((Date.now() - new Date(iso).getTime()) / 1000));
  if (secs < 60) return `${secs}s ago`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  return `${Math.floor(secs / 3600)}h ago`;
}
