// SPDX-License-Identifier: AGPL-3.0-only

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [["year", 31_536_000], ["month", 2_592_000], ["week", 604_800], ["day", 86_400], ["hour", 3_600], ["minute", 60]];
const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

/** "3 minutes ago", "in 2 hours", "just now". */
export function ago(value?: string | number | Date | null, now = Date.now()): string {
  if (value == null || value === "") return "never";
  const at = new Date(value).valueOf();
  if (Number.isNaN(at)) return String(value);
  const seconds = (at - now) / 1000;
  for (const [unit, size] of UNITS) {
    if (Math.abs(seconds) >= size) return formatter.format(Math.round(seconds / size), unit);
  }
  return Math.abs(seconds) < 45 ? "just now" : formatter.format(Math.round(seconds / 60), "minute");
}

/** "every 15 min", "every 6 h", "every 2 days". */
export function interval(ms: number): string {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 1) return `every ${Math.max(1, Math.round(ms / 1000))} s`;
  if (minutes < 60) return `every ${minutes} min`;
  if (minutes < 1440) return `every ${Math.round(minutes / 60)} h`;
  const days = Math.round(minutes / 1440);
  return days === 1 ? "daily" : `every ${days} days`;
}
