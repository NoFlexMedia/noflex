// SPDX-License-Identifier: AGPL-3.0-only
//
// Formatting for the Activity tab's play statistics.

export const RANGES = [
  { value: "LAST_24_HOURS", label: "24h", hint: "Last 24 hours", long: "the last 24 hours" },
  { value: "LAST_7_DAYS", label: "7d", hint: "Last 7 days", long: "the last 7 days" },
  { value: "LAST_30_DAYS", label: "30d", hint: "Last 30 days", long: "the last 30 days" },
  { value: "LAST_90_DAYS", label: "90d", hint: "Last 90 days", long: "the last 90 days" },
  { value: "LAST_YEAR", label: "1y", hint: "Last year", long: "the last year" },
  { value: "ALL_TIME", label: "All", hint: "All time", long: "all time" },
] as const;

/** The browser's current offset, which the server aligns buckets to. */
export const utcOffset = (): number => -new Date().getTimezoneOffset();

/** 1200 → "1.2K". */
export function compact(value: number): string {
  return new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(value);
}

/** Watched time for humans: "—", "12 min", "3.4 h", "128 h". */
export function hours(ms?: number | null): string {
  if (!ms) return "0 min";
  const minutes = ms / 60_000;
  if (minutes < 1) return "<1 min";
  if (minutes < 60) return `${Math.round(minutes)} min`;
  const h = minutes / 60;
  return `${h < 10 ? h.toFixed(1) : compact(Math.round(h))} h`;
}

/** Axis (short) or tooltip (long) label for a bucket start. */
export function bucketLabel(start: string, bucket: string, long = false): string {
  const at = new Date(start);
  const fmt = (options: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat(undefined, options).format(at);
  switch (bucket) {
    case "HOUR":
      return long ? fmt({ weekday: "short", hour: "numeric" }) : fmt({ hour: "numeric" });
    case "DAY":
      return long ? fmt({ weekday: "long", month: "short", day: "numeric" }) : fmt({ month: "short", day: "numeric" });
    case "WEEK":
      return long ? `Week of ${fmt({ month: "short", day: "numeric", year: "numeric" })}` : fmt({ month: "short", day: "numeric" });
    default:
      return long ? fmt({ month: "long", year: "numeric" }) : fmt({ month: "short", year: at.getMonth() === 0 ? "2-digit" : undefined });
  }
}

/** "S1 · E3 · Title" for episodes, the plain title otherwise. */
export function playedTitle(work: { kind: string; title: string; parentIndex?: number | null; indexNumber?: number | null } | null | undefined): string {
  if (!work) return "Removed title";
  if (work.kind === "EPISODE" && work.indexNumber != null) return `S${work.parentIndex ?? 0} · E${work.indexNumber} · ${work.title}`;
  return work.title;
}
