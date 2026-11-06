// SPDX-License-Identifier: AGPL-3.0-only
//
// Display formatting, ported from web/src/lib/format.ts. What the TV doesn't draw stays there.

export function titleCase(value = ""): string {
  return value.toLowerCase().split("_").join(" ").replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export function runtime(milliseconds?: number | null): string {
  if (!milliseconds) return "";
  const minutes = Math.round(milliseconds / 60000);
  return minutes >= 60 ? `${Math.floor(minutes / 60)}h ${minutes % 60}m` : `${minutes}m`;
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

export function dynamicRange(video: any): string {
  const transfer = String(video?.transfer ?? "").toLowerCase();
  return video?.dovi_profile != null ? "Dolby Vision" : transfer === "pq" ? "HDR10" : transfer === "hlg" ? "HLG" : "";
}

const AUDIO_NAMES: Record<string, string> = { aac: "AAC", ac3: "AC-3", eac3: "E-AC-3", truehd: "TrueHD", dts: "DTS", flac: "FLAC", opus: "Opus", vorbis: "Vorbis", mp3: "MP3", pcm_s16le: "PCM", pcm_s24le: "PCM" };
const CHANNEL_NAMES: Record<number, string> = { 1: "Mono", 2: "Stereo", 6: "5.1", 8: "7.1" };

export function audioFormat(track: any): string {
  const codec = String(track?.codec ?? "").toLowerCase();
  return [AUDIO_NAMES[codec] ?? codec.toUpperCase(), CHANNEL_NAMES[track?.channels] ?? (track?.channels ? `${track.channels} ch` : "")].filter(Boolean).join(" ");
}

/** What a file's picture and chosen sound are, in a few badges: resolution, codec, HDR, audio. */
export function techBadges(probe: any, audio: any): string[] {
  const video = probe?.video;
  const height = video?.height;
  const tier = !height ? "" : height >= 2000 ? "4K" : height >= 1400 ? "1440p" : height >= 1000 ? "1080p" : height >= 700 ? "720p" : `${height}p`;
  const name = String(video?.codec ?? "").toLowerCase();
  return [tier, CODEC_NAMES[name] ?? name.toUpperCase(), video ? dynamicRange(video) : "", audio ? audioFormat(audio) : ""].filter(Boolean);
}

export function bytes(size?: number | null): string {
  if (!size) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const power = Math.min(units.length - 1, Math.floor(Math.log(size) / Math.log(1024)));
  return `${(size / 1024 ** power).toFixed(power > 2 ? 1 : 0)} ${units[power]}`;
}

/** A codec enum or ffprobe name as viewers know it: `H265` → "HEVC". */
export function codecName(code?: string | null): string {
  const name = String(code ?? "").toLowerCase();
  return CODEC_NAMES[name] ?? name.toUpperCase();
}

/** A level in the server's (ffprobe's) units: H.264 ×10, HEVC ×30, AV1 `seq_level_idx`. */
export function levelName(codec?: string | null, level?: number | null): string | null {
  if (level == null || level <= 0) return null;
  const name = String(codec ?? "").toLowerCase();
  return name === "h264" ? `L${level / 10}` : name === "h265" || name === "hevc" ? `L${Math.round(level / 3) / 10}` : `L${level}`;
}

/** A profile enum as it's usually written: `HEVC_MAIN10` → "Main 10". */
export function profileName(profile?: string | null): string {
  if (!profile) return "";
  return profile.replace(/^(H264|HEVC|AV1|VP9)_/i, "").replace(/(\D)(\d+)$/, "$1 $2").replace(/_/g, " ").toLowerCase().replace(/^\w/, (c) => c.toUpperCase());
}

/** A media file's version as viewers name it: "4K · HEVC · HDR10". Null when the probe has no video. */
export function versionLabel(probe: any): string | null {
  const video = probe?.video;
  const height = video?.height;
  if (!height) return null;
  const tier = height >= 2000 ? "4K" : height >= 1400 ? "1440p" : height >= 1000 ? "1080p" : height >= 700 ? "720p" : `${height}p`;
  const codec = CODEC_NAMES[String(video.codec ?? "").toLowerCase()] ?? String(video.codec ?? "").toUpperCase();
  return [tier, codec, dynamicRange(video)].filter(Boolean).join(" · ");
}

const ISO639_2: Record<string, string> = { eng: "en", spa: "es", fre: "fr", fra: "fr", ger: "de", deu: "de", ita: "it", jpn: "ja", kor: "ko", chi: "zh", zho: "zh", por: "pt", rus: "ru", dut: "nl", nld: "nl", swe: "sv", nor: "no", dan: "da", fin: "fi", pol: "pl", tur: "tr", ara: "ar", hin: "hi" };

/** A language tag in the one spelling used to compare them: `eng` and `EN` → `en`. */
export function languageCode(code: string): string {
  const lower = code.trim().toLowerCase();
  return ISO639_2[lower] ?? lower;
}

// Older TV engines have no Intl.DisplayNames, so the commonest names travel with the app.
const LANGUAGE_NAMES: Record<string, string> = { en: "English", es: "Spanish", fr: "French", de: "German", it: "Italian", ja: "Japanese", ko: "Korean", zh: "Chinese", pt: "Portuguese", ru: "Russian", nl: "Dutch", sv: "Swedish", no: "Norwegian", da: "Danish", fi: "Finnish", pl: "Polish", tr: "Turkish", ar: "Arabic", hi: "Hindi" };
const displayNames = (() => { try { return new (Intl as any).DisplayNames(undefined, { type: "language" }); } catch { return null; } })();

export function languageName(code?: string | null): string {
  if (!code) return "";
  const normal = languageCode(code);
  try { return displayNames?.of(normal) ?? LANGUAGE_NAMES[normal] ?? code.toUpperCase(); } catch { return LANGUAGE_NAMES[normal] ?? code.toUpperCase(); }
}
