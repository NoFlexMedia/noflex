// SPDX-License-Identifier: AGPL-3.0-only
//
// Number formatting for the diagnostics tabs. Unlike `$lib/format`, zero
// is a real reading here ("0 B", "0%"), and only null means unknown.

const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

/** 1536 → "1.5 KB"; null → "—". Binary multiples, as `df` and the OS report them. */
export function size(value?: number | null, digits = 1): string {
  if (value == null || Number.isNaN(value)) return "—";
  if (value < 1024) return `${Math.round(value)} B`;
  const power = Math.min(Math.floor(Math.log(value) / Math.log(1024)), UNITS.length - 1);
  return `${(value / 1024 ** power).toFixed(power >= 3 ? digits : 0)} ${UNITS[power]}`;
}

/** Bytes per second as bits per second, the way links are rated: "12.4 Mb/s". */
export function rate(bytesPerSec?: number | null): string {
  if (bytesPerSec == null) return "—";
  const bits = bytesPerSec * 8;
  if (bits < 1000) return `${Math.round(bits)} b/s`;
  const units = ["Kb/s", "Mb/s", "Gb/s"];
  const power = Math.min(Math.floor(Math.log10(bits) / 3), units.length);
  return `${(bits / 1000 ** power).toFixed(bits / 1000 ** power < 10 ? 1 : 0)} ${units[power - 1]}`;
}

export function percent(value?: number | null, digits = 0): string {
  return value == null || Number.isNaN(value) ? "—" : `${value.toFixed(digits)}%`;
}

export function ratio(part?: number | null, whole?: number | null): number {
  return part == null || !whole ? 0 : Math.min(100, Math.max(0, (part / whole) * 100));
}

/** 93784 s → "1d 2h", 3720 → "1h 2m", 42 → "42s". */
export function uptime(seconds?: number | null): string {
  if (seconds == null) return "—";
  const d = Math.floor(seconds / 86400);
  const h = Math.floor(seconds / 3600) % 24;
  const m = Math.floor(seconds / 60) % 60;
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${Math.floor(seconds % 60)}s`;
  return `${Math.floor(seconds)}s`;
}

export function bitrate(bps?: number | null): string {
  if (!bps) return "—";
  return bps >= 1_000_000 ? `${(bps / 1_000_000).toFixed(1)} Mb/s` : `${Math.round(bps / 1000)} kb/s`;
}

/** Severity for a utilisation percentage: drives meter colour. */
export function tone(pct: number, warn = 80, bad = 92): "ok" | "warn" | "bad" {
  return pct >= bad ? "bad" : pct >= warn ? "warn" : "ok";
}

/** Fixed-width 24-hour time of day, so timestamp columns line up. */
export function clock(value?: string | null): string {
  if (!value) return "—";
  const at = new Date(value);
  return Number.isNaN(at.valueOf()) ? value : at.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hourCycle: "h23" });
}

export function stamp(value?: string | null): string {
  if (!value) return "—";
  const at = new Date(value);
  return Number.isNaN(at.valueOf()) ? value : at.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

const CODEC: Record<string, string> = { H264: "H.264", H265: "HEVC", AV1: "AV1", VP9: "VP9", VP8: "VP8", MPEG2: "MPEG-2", PRORES: "ProRes", THEORA: "Theora" };
export const codec = (value?: string | null) => (value ? CODEC[value] ?? value : "—");

const RANGE: Record<string, string> = { SDR: "SDR", HDR10: "HDR10", HDR10_PLUS: "HDR10+", HLG: "HLG", DOVI: "Dolby Vision", DOVI_WITH_SDR: "DV · SDR", DOVI_WITH_HDR10: "DV · HDR10", DOVI_WITH_HLG: "DV · HLG" };
export const range = (value?: string | null) => (value ? RANGE[value] ?? value : "—");

export function channels(count?: number | null): string {
  if (!count) return "";
  return ({ 1: "mono", 2: "stereo", 6: "5.1", 8: "7.1" } as Record<number, string>)[count] ?? `${count} ch`;
}

export function resolution(width?: number | null, height?: number | null): string {
  if (!width || !height) return "—";
  const label = height >= 2000 ? "4K" : height >= 1400 ? "1440p" : height >= 1000 ? "1080p" : height >= 700 ? "720p" : height >= 560 ? "576p" : height >= 460 ? "480p" : `${height}p`;
  return `${label} · ${width}×${height}`;
}
