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

/** What HDR a probed video stream carries, as viewers name it; "" for SDR. */
function dynamicRange(video: any): string {
  const transfer = String(video?.transfer ?? "").toLowerCase();
  return video?.dovi_profile != null ? "Dolby Vision" : transfer === "pq" ? "HDR10" : transfer === "hlg" ? "HLG" : "";
}

const AUDIO_NAMES: Record<string, string> = { aac: "AAC", ac3: "AC-3", eac3: "E-AC-3", truehd: "TrueHD", dts: "DTS", flac: "FLAC", opus: "Opus", vorbis: "Vorbis", mp3: "MP3", pcm_s16le: "PCM", pcm_s24le: "PCM" };
const CHANNEL_NAMES: Record<number, string> = { 1: "Mono", 2: "Stereo", 6: "5.1", 8: "7.1" };

/** An audio track's format as viewers name it, e.g. "E-AC-3 5.1". */
export function audioFormat(track: any): string {
  const codec = String(track?.codec ?? "").toLowerCase();
  return [AUDIO_NAMES[codec] ?? codec.toUpperCase(), CHANNEL_NAMES[track?.channels] ?? (track?.channels ? `${track.channels} ch` : "")].filter(Boolean).join(" ");
}

/**
 * What a file's picture and chosen sound are, in a few badges: resolution,
 * codec, HDR, audio. `edition` stands in where the file's probe is missing.
 */
export function techBadges(probe: any, edition: any, audio: any): string[] {
  const video = probe?.video;
  const height = video?.height ?? edition?.height;
  const tier = !height ? "" : height >= 2000 ? "4K" : height >= 1400 ? "1440p" : height >= 1000 ? "1080p" : height >= 700 ? "720p" : `${height}p`;
  const name = String(video?.codec ?? edition?.videoCodec ?? edition?.codecVideo ?? "").toLowerCase();
  const range = video ? dynamicRange(video) : edition?.hdrFormat ? (/dolby|dovi/i.test(edition.hdrFormat) ? "Dolby Vision" : edition.hdrFormat.toUpperCase().includes("HLG") ? "HLG" : "HDR") : "";
  return [tier, CODEC_NAMES[name] ?? name.toUpperCase(), range, audio ? audioFormat(audio) : ""].filter(Boolean);
}

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
  const range = dynamicRange(video);
  return [tier, codec, range].filter(Boolean).join(" · ");
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
  return profile.replace(/^(H264|HEVC|AV1|VP9)_/, "").replace(/(\D)(\d+)$/, "$1 $2").replace(/_/g, " ").toLowerCase().replace(/^\w/, (c) => c.toUpperCase());
}

/** A picture area as the resolution people call it, e.g. 8294400 → "~4K". */
export function pixelTier(pixels: number): string {
  const tiers: [number, string][] = [[33_177_600, "8K"], [14_745_600, "5K"], [8_294_400, "4K"], [3_686_400, "1440p"], [2_073_600, "1080p"], [921_600, "720p"], [409_920, "480p"], [230_400, "360p"]];
  const [, name] = tiers.find(([size]) => pixels >= size * 0.9) ?? [0, `${Math.round(Math.sqrt((pixels * 9) / 16))}p`];
  return `~${name}`;
}

const languages = (() => { try { return new Intl.DisplayNames(undefined, { type: "language" }); } catch { return null; } })();

/** "en" / "eng" / "pt-BR" → "English" / "Portuguese (Brazil)"; unknown codes come back uppercased. */
const ISO639_2: Record<string, string> = { eng: "en", spa: "es", fre: "fr", fra: "fr", ger: "de", deu: "de", ita: "it", jpn: "ja", kor: "ko", chi: "zh", zho: "zh", por: "pt", rus: "ru", dut: "nl", nld: "nl", swe: "sv", nor: "no", dan: "da", fin: "fi", pol: "pl", tur: "tr", ara: "ar", hin: "hi" };

/** A language tag in the one spelling used to compare them: `eng` and `EN` → `en`. */
export function languageCode(code: string): string {
  const lower = code.trim().toLowerCase();
  return ISO639_2[lower] ?? lower;
}

export function languageName(code?: string | null): string {
  if (!code) return "";
  const normalised = ISO639_2[code.toLowerCase()] ?? code;
  try { return languages?.of(normalised) ?? code.toUpperCase(); } catch { return code.toUpperCase(); }
}
