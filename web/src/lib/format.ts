// SPDX-License-Identifier: AGPL-3.0-only

export function titleCase(value = ""): string {
  return value.toLowerCase().replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export function runtime(milliseconds?: number | null): string {
  if (!milliseconds) return "";
  const minutes = Math.round(milliseconds / 60000);
  return minutes >= 60 ? `${Math.floor(minutes / 60)}h ${minutes % 60}m` : `${minutes}m`;
}

export function date(value?: string | null, options: Intl.DateTimeFormatOptions = { dateStyle: "medium" }): string {
  if (!value) return "—";
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf()) ? value : new Intl.DateTimeFormat(undefined, options).format(parsed);
}

export function bytes(value?: number | null): string {
  if (!value) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const power = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / (1024 ** power)).toFixed(power > 2 ? 1 : 0)} ${units[power]}`;
}

export function splitList(value: FormDataEntryValue | string | null): string[] {
  return String(value ?? "").split(/[\n,]/).map((item) => item.trim()).filter(Boolean);
}

export function optionalNumber(value: string | number | null | undefined): number | null {
  return value === "" || value == null ? null : Number(value);
}

/** Player-style clock: "4:05", "1:02:03". */
export function timecode(milliseconds: number): string {
  const total = Math.max(0, Math.floor(milliseconds / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const seconds = String(total % 60).padStart(2, "0");
  return hours ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}

const CODEC_NAMES: Record<string, string> = { h264: "H.264", h265: "HEVC", hevc: "HEVC", av1: "AV1", vp9: "VP9", mpeg2video: "MPEG-2", vc1: "VC-1" };

/**
 * A media file's version as viewers name it, from its ffprobe report:
 * "4K · HEVC · HDR10", "1080p · AV1". Null when the probe has no video.
 */
export function versionLabel(probe: any): string | null {
  const video = probe?.video;
  const height = video?.height;
  if (!height) return null;
  const tier = height >= 2000 ? "4K" : height >= 1400 ? "1440p" : height >= 1000 ? "1080p" : height >= 700 ? "720p" : `${height}p`;
  const codec = CODEC_NAMES[String(video.codec ?? "").toLowerCase()] ?? String(video.codec ?? "").toUpperCase();
  const transfer = String(video.transfer ?? "").toLowerCase();
  const range = video.dovi_profile != null ? "Dolby Vision" : transfer.includes("2084") ? "HDR10" : transfer.includes("b67") ? "HLG" : "";
  return [tier, codec, range].filter(Boolean).join(" · ");
}

const languages = (() => { try { return new Intl.DisplayNames(undefined, { type: "language" }); } catch { return null; } })();

/** "en" / "eng" / "pt-BR" → "English" / "Portuguese (Brazil)"; unknown codes come back uppercased. */
export function languageName(code?: string | null): string {
  if (!code) return "";
  const ISO639_2: Record<string, string> = { eng: "en", spa: "es", fre: "fr", fra: "fr", ger: "de", deu: "de", ita: "it", jpn: "ja", kor: "ko", chi: "zh", zho: "zh", por: "pt", rus: "ru", dut: "nl", nld: "nl", swe: "sv", nor: "no", dan: "da", fin: "fi", pol: "pl", tur: "tr", ara: "ar", hin: "hi" };
  const normalised = ISO639_2[code.toLowerCase()] ?? code;
  try { return languages?.of(normalised) ?? code.toUpperCase(); } catch { return code.toUpperCase(); }
}
