// SPDX-License-Identifier: AGPL-3.0-only
//
// What this browser can decode, measured rather than assumed, in the
// shape of the server's `ClientCapabilities` input.

export type VideoCodec = "H264" | "H265" | "AV1" | "VP9" | "VP8";
export type AudioCodec = "AAC" | "AC3" | "EAC3" | "MP3" | "OPUS" | "VORBIS" | "FLAC" | "PCM";
export type VideoProfile =
  | "H264_HIGH10" | "H264_HIGH" | "H264_MAIN" | "H264_BASELINE" | "H264_CONSTRAINED_BASELINE"
  | "HEVC_MAIN10" | "HEVC_MAIN"
  | "AV1_PROFESSIONAL" | "AV1_HIGH" | "AV1_MAIN"
  | "VP9_PROFILE3" | "VP9_PROFILE2" | "VP9_PROFILE1" | "VP9_PROFILE0";
export type VideoRange = "SDR" | "HDR10" | "HDR10_PLUS" | "HLG" | "DOVI" | "DOVI_WITH_SDR" | "DOVI_WITH_HDR10" | "DOVI_WITH_HLG";

export interface DirectPlayProfile { container: string; videoCodecs: VideoCodec[]; audioCodecs: AudioCodec[] }
export interface HlsProfile { segmentContainer: "FMP4" | "TS"; videoCodecs: VideoCodec[]; audioCodecs: AudioCodec[] }
export interface VideoCodecSupport {
  codec: VideoCodec;
  profiles: VideoProfile[];
  maxLevel: number | null;
  maxBitDepth: number | null;
  maxWidth: number | null;
  maxHeight: number | null;
  /** Largest picture in luma samples, which is what levels actually limit. */
  maxPixels: number | null;
  videoRanges: VideoRange[];
}
export interface AudioCodecSupport { codec: AudioCodec; maxChannels: number | null }
export interface ClientCapabilities {
  directPlay: DirectPlayProfile[];
  hls: HlsProfile[];
  videoCodecs: VideoCodecSupport[];
  audioCodecs: AudioCodecSupport[];
  subtitleCodecs: "WEB_VTT"[];
  maxHeight: number | null;
  maxBitrateBps: number | null;
}

export interface Resolution { width: number; height: number }

// ---- pure parts -----------------------------------------------------------

/** The container MIME the helper probes each codec under. */
export function mimeFor(codec: VideoCodec): string {
  return codec === "H264" || codec === "H265" ? "video/mp4" : "video/webm";
}

/**
 * RFC 6381 string for `codec`. `level` is in the server's (ffprobe's)
 * units: H.264 level×10, HEVC level×30, AV1 seq_level_idx, VP9 level×10.
 * Defaults are the lowest profile/level, as in the helper.
 */
export function codecString(codec: VideoCodec, profile?: VideoProfile, level?: number, bitDepth?: number): string {
  switch (codec) {
    case "H264": {
      const flag = { H264_HIGH10: "6e00", H264_HIGH: "6400", H264_MAIN: "4d00", H264_CONSTRAINED_BASELINE: "4240" }[profile as string] ?? "4200";
      // Levels are bound by max frame size and macroblock rate.
      return `avc1.${flag}${(level ?? 10).toString(16).padStart(2, "0")}`;
    }
    case "H265": {
      // The helper's `hev1.1.<flag>.L<level>` always names profile_idc 1
      // (Main), so a Main 10 probe never tested Main 10; spell the real
      // profile_idc and compatibility flags instead. Levels are bound by
      // luma picture size and sample rate.
      const [idc, compat] = profile === "HEVC_MAIN10" ? [2, 4] : [1, 6];
      return `hvc1.${idc}.${compat}.L${level ?? 30}.B0`;
    }
    case "VP8":
      return "vp8";
    case "VP9": {
      const p = { VP9_PROFILE1: "01", VP9_PROFILE2: "02", VP9_PROFILE3: "03" }[profile as string] ?? "00";
      return `vp09.${p}.${String(level ?? 10).padStart(2, "0")}.${String(bitDepth ?? 8).padStart(2, "0")}`;
    }
    case "AV1": {
      const p = { AV1_HIGH: "1", AV1_PROFESSIONAL: "2" }[profile as string] ?? "0";
      // Main tier: the capability model has no way to express tier.
      return `av01.${p}.${String(level ?? 0).padStart(2, "0")}M.${String(bitDepth ?? 8).padStart(2, "0")}`;
    }
  }
}

/**
 * Candidate profiles, most capable first. Order matters: the server
 * probes level and bit depth on the first supported profile, so a
 * baseline-first list would cap every copy at baseline limits.
 */
export function profileCandidates(codec: VideoCodec): (VideoProfile | undefined)[] {
  switch (codec) {
    case "H264": return ["H264_HIGH10", "H264_HIGH", "H264_MAIN", "H264_BASELINE", "H264_CONSTRAINED_BASELINE"];
    // HEVC "high" / "high 10" in the helper are tiers, not profiles, and
    // the server doesn't rank them; leave them out rather than lead with
    // them.
    case "H265": return ["HEVC_MAIN10", "HEVC_MAIN"];
    case "AV1": return ["AV1_PROFESSIONAL", "AV1_HIGH", "AV1_MAIN"];
    case "VP9": return ["VP9_PROFILE3", "VP9_PROFILE2", "VP9_PROFILE1", "VP9_PROFILE0"];
    case "VP8": return [undefined];
  }
}

/** Level table in server units, ascending. */
export function levelTable(codec: VideoCodec): number[] {
  switch (codec) {
    case "H264": return [10, 11, 12, 13, 20, 21, 22, 30, 31, 32, 40, 41, 42, 50, 51, 52, 60, 61, 62];
    // The server expects H.265 levels multiplied by 3 (general_level_idc).
    case "H265": return [10, 20, 21, 30, 31, 40, 41, 50, 51, 52, 60, 61, 62].map((level) => level * 3);
    // seq_level_idx.
    case "AV1": return [0, 1, 4, 5, 8, 9, 12, 13, 14, 15, 16, 17, 18, 19];
    case "VP9": return [10, 11, 20, 21, 30, 31, 40, 41, 50, 51, 52, 60, 61, 62];
    case "VP8": return [];
  }
}

// Largest picture per level, in luma samples. H.264 specifies macroblocks
// (MaxFS × 256); HEVC, AV1 and VP9 specify samples directly.
const H264_MAX_FS: Record<number, number> = { 10: 99, 11: 396, 12: 396, 13: 396, 20: 396, 21: 792, 22: 1620, 30: 1620, 31: 3600, 32: 5120, 40: 8192, 41: 8192, 42: 8704, 50: 22080, 51: 36864, 52: 36864, 60: 139264, 61: 139264, 62: 139264 };
const HEVC_MAX_LUMA_PS: Record<number, number> = { 30: 36864, 60: 122880, 63: 245760, 90: 552960, 93: 983040, 120: 2228224, 123: 2228224, 150: 8912896, 153: 8912896, 156: 8912896, 180: 35651584, 183: 35651584, 186: 35651584 };
const VP9_MAX_LUMA_PS: Record<number, number> = { 10: 36864, 11: 73728, 20: 122880, 21: 245760, 30: 552960, 31: 983040, 40: 2228224, 41: 2228224, 50: 8912896, 51: 8912896, 52: 8912896, 60: 35651584, 61: 35651584, 62: 35651584 };
// seq_level_idx 0–3 are 2.x, 4–7 are 3.x, 8–11 are 4.x, 12–15 are 5.x, 16+ are 6.x.
const AV1_MAX_PIC_SIZE: Record<number, number> = { 0: 147456, 1: 278784, 4: 665856, 5: 1065024, 8: 2359296, 9: 2359296, 12: 8912896, 13: 8912896, 14: 8912896, 15: 8912896, 16: 35651584, 17: 35651584, 18: 35651584, 19: 35651584 };

/** Largest picture a decoder of `codec` at `level` (server units) must handle. */
export function levelMaxPixels(codec: VideoCodec, level: number | undefined): number | null {
  if (level == null) return null;
  switch (codec) {
    case "H264": return H264_MAX_FS[level] != null ? H264_MAX_FS[level] * 256 : null;
    case "H265": return HEVC_MAX_LUMA_PS[level] ?? null;
    case "AV1": return AV1_MAX_PIC_SIZE[level] ?? null;
    case "VP9": return VP9_MAX_LUMA_PS[level] ?? null;
    case "VP8": return null;
  }
}

/** Candidate bit depths for a profile, highest first. */
export function bitDepthCandidates(codec: VideoCodec, profile?: VideoProfile): number[] {
  switch (codec) {
    case "H264": return profile === "H264_HIGH10" ? [10, 8] : [8];
    case "H265": return profile === "HEVC_MAIN10" ? [10, 8] : [8];
    case "AV1": return profile === "AV1_PROFESSIONAL" ? [12, 10, 8] : [10, 8];
    // VP9 profiles 2 and 3 require 10 bits or more.
    case "VP9": return profile === "VP9_PROFILE2" || profile === "VP9_PROFILE3" ? [12, 10] : [8];
    // VP8's bitstream only goes to 8 bits.
    case "VP8": return [8];
  }
}

/**
 * Profiles whose codec string `supports` accepts at some bit depth, in
 * candidate order. Any depth will do: Firefox takes AV1 Professional at 12
 * bits but not 8, though either makes it the same profile.
 */
export function supportedProfiles(codec: VideoCodec, supports: (codecs: string) => boolean): (VideoProfile | undefined)[] {
  return profileCandidates(codec).filter((profile) => bitDepthCandidates(codec, profile).some((depth) => supports(codecString(codec, profile, undefined, depth))));
}

/** Highest level reached by walking the table until the first failure. */
export function highestLevel(codec: VideoCodec, profile: VideoProfile | undefined, bitDepth: number | undefined, supports: (codecs: string) => boolean): number | undefined {
  let highest: number | undefined;
  for (const level of levelTable(codec)) {
    if (!supports(codecString(codec, profile, level, bitDepth))) break;
    highest = level;
  }
  return highest;
}

export function highestBitDepth(codec: VideoCodec, profile: VideoProfile | undefined, level: number | undefined, supports: (codecs: string) => boolean): number | undefined {
  return bitDepthCandidates(codec, profile).find((depth) => supports(codecString(codec, profile, level, depth)));
}

/**
 * Lowest Dolby Vision level whose max pixel rate covers the base-layer
 * `level` (server units). Tables from the helper; mirrored server-side
 * in `noflex-transcoder/src/hls.rs` for SUPPLEMENTAL-CODECS.
 */
export function doviLevel(codec: "H264" | "H265" | "AV1", level: number): string {
  // [max base level, DoVi level]
  const tables: Record<"H264" | "H265" | "AV1", [number, string][]> = {
    // HEVC level | HEVC max PPS  | DoVi level | DoVi max PPS
    // 3.0        |    16_588_800 | 01         |    22_118_400
    // 3.1        |    33_177_600 | 03         |    49_766_400
    // 4.0        |    66_846_720 | 04         |   124_416_000
    // 4.1        |   133_693_440 | 06         |   199_065_600
    // 5.0        |   267_386_880 | 07         |   248_832_000
    // 5.1        |   534_773_760 | 10         |   995_328_000
    // 6.0        | 1_069_547_520 | 11         | 1_990_656_000
    // 6.1        | 2_139_095_040 | 13         | 3_981_312_000
    H265: [[90, "01"], [93, "03"], [120, "04"], [123, "06"], [150, "07"], [153, "10"], [180, "11"]],
    // AV1 seq_level_idx | AV1 max PPS | DoVi level
    // 4: 19_975_680 → 01, 5: 31_950_720 → 03, 8: 70_778_880 → 04,
    // 9: 141_557_760 → 06, 12: 267_386_880 → 07, 13: 534_773_760 → 10,
    // 16: 1_069_547_520 → 11, beyond → 13
    AV1: [[4, "01"], [5, "03"], [8, "04"], [9, "06"], [12, "07"], [13, "10"], [16, "11"]],
    // H.264 max PPS assuming the largest (16×16) macroblock:
    // 3.0: 10_368_000 → 01, 3.1: 27_648_000 → 02, 4.0/4.1: 62_914_560 → 05,
    // 4.2/5.0: ≤150_994_944 → 06, 5.1: 251_658_240 → 08,
    // 5.2: 530_841_600 → 10, 6.0: 1_069_547_520 → 12, beyond → 13
    H264: [[30, "01"], [31, "02"], [41, "05"], [50, "06"], [51, "08"], [52, "10"], [60, "12"]],
  };
  return tables[codec].find(([max]) => level <= max)?.[1] ?? "13";
}

/**
 * The most a decoder is good for: what it answered for 16:9, bounded by what
 * its level guarantees. Browsers call everything up to 8K supported and smooth
 * without looking at the level, while hardware's real limit shows in the level
 * alone, so neither answer stands by itself.
 */
export function maxPixels(max: Resolution | null, levelLimit: number | null): number | null {
  const probed = max ? max.width * max.height : null;
  return probed != null && levelLimit != null ? Math.min(probed, levelLimit) : probed ?? levelLimit;
}

/**
 * 16:9 candidates up to 8K: the helper's progressively coarser steps,
 * stopped at 7680 wide (it goes to 30720) to bound the probe count.
 */
export function resolutionSteps(): Resolution[] {
  const steps: Resolution[] = [];
  let width = 0, height = 0;
  while (width < 7680) {
    if (height >= 2160) { width += 1280; height += 720; }
    else if (height >= 1080) { width += 640; height += 360; }
    else { width += 320; height += 180; }
    steps.push({ width, height });
  }
  return steps;
}

/**
 * Other aspect ratios to try once the 16:9 maximum is known, ordered by
 * the scaling factor of the expanding dimension — 2.40:1 before 1.85:1,
 * wider before taller. One dimension is held, the other expanded.
 */
export function alternativeResolutions(max: Resolution): Resolution[] {
  const { width, height } = max;
  return [
    { width: Math.floor(height * 3.555), height }, // 32:9 super ultrawide
    { width: Math.floor(height * 2.4), height }, // 2.40:1 35 mm scope
    { width: Math.floor(height * 2.37037), height }, // "21:9" (really ~2.37:1)
    { width: Math.floor(height * 1.9), height }, // 1.90:1 IMAX
    { width: Math.floor(height * 1.85), height }, // 1.85:1 flat
    { width, height: Math.floor(width / 9) * 19.5 }, // 9:19.5 phone
    { width, height: Math.floor(width / 9) * 16 }, // 9:16
    { width, height: width }, // 1:1
    { width, height: Math.floor((width / 4) * 3) }, // 4:3
    { width, height: Math.floor((width / 16) * 10) }, // 16:10
  ];
}

/** Rough bitrate for a probe at `res`; decodingInfo requires one. */
export function probeBitrate(res: Resolution, framerate = 30): number {
  return Math.round(res.width * res.height * framerate * 0.13);
}

// ---- browser probes -------------------------------------------------------

type Probe = (type: string) => boolean;

function makeProbes() {
  const video = document.createElement("video");
  const canPlay: Probe = (type) => video.canPlayType(type) !== "";
  // iOS Safari only has ManagedMediaSource; hls.js uses it there too.
  const Source: any = (window as any).ManagedMediaSource ?? (window as any).MediaSource;
  const mse: Probe | null = Source?.isTypeSupported ? (type) => !!Source.isTypeSupported(type) : null;
  return { canPlay, mse, nativeHls: canPlay("application/vnd.apple.mpegurl") };
}

const typeFor = (mime: string, codecs: string) => `${mime}; codecs="${codecs}"`;

// decodingInfo is fast where it's backed by a static table but can hit
// the GPU process elsewhere; never let one probe hold up playback.
async function decodes(config: any): Promise<{ supported: boolean; smooth: boolean } | null> {
  const caps = navigator.mediaCapabilities;
  if (!caps?.decodingInfo) return null;
  try {
    const timeout = new Promise<null>((resolve) => setTimeout(() => resolve(null), 400));
    return await Promise.race([caps.decodingInfo(config).then((r) => ({ supported: r.supported, smooth: r.smooth })), timeout]);
  } catch {
    return null;
  }
}

/** Whether the display path can present HDR at all. */
export function displayHasHdr(): boolean {
  try {
    return matchMedia("(dynamic-range: high)").matches || matchMedia("(video-dynamic-range: high)").matches;
  } catch {
    return false;
  }
}

async function maxResolution(mime: string, codecs: string, type: "media-source" | "file"): Promise<{ standard: Resolution; widest: Resolution } | null> {
  const probe = (res: Resolution) =>
    decodes({ type, video: { contentType: typeFor(mime, codecs), width: res.width, height: res.height, bitrate: probeBitrate(res), framerate: 30 } })
      // `supported` alone is true for anything software can grind
      // through; above 1080p also demand realtime.
      .then((r) => (r ? r.supported && (r.smooth || res.height <= 1080) : null));
  const steps = resolutionSteps();
  const results = await Promise.all(steps.map(probe));
  if (results.every((r) => r === null)) return null;
  // Like the helper, keep the last passing step rather than stopping at
  // the first failure: some decoders reject odd intermediate sizes.
  let max: Resolution = { width: 0, height: 0 };
  steps.forEach((step, i) => { if (results[i]) max = step; });
  if (!max.width) return null;
  const alternatives = alternativeResolutions(max);
  const passed = await Promise.all(alternatives.map(probe));
  return { standard: max, widest: alternatives[passed.findIndex(Boolean)] ?? max };
}

async function videoRanges(codec: VideoCodec, profile: VideoProfile | undefined, level: number, supports: Probe, type: "media-source" | "file"): Promise<VideoRange[]> {
  const ranges = new Set<VideoRange>(["SDR"]);
  const mime = mimeFor(codec);
  const base = codecString(codec, profile, level, 10);
  const tenBit = codec === "H265" ? profile === "HEVC_MAIN10" : codec === "AV1" || (codec === "VP9" && (profile === "VP9_PROFILE2" || profile === "VP9_PROFILE3"));
  if (tenBit && displayHasHdr()) {
    // AV1 and VP9 codec strings carry colour fields, so the transfer
    // function is part of the type itself; HEVC has none and relies on
    // decodingInfo alone. The helper's AV1 form omits the monochrome
    // digit (`…10.110…`), which no parser accepts; this spells it.
    //   AV1: 0 mono, 110 4:2:0, 09 BT.2020, 16 PQ / 18 HLG, 09 BT.2020 NCL, 0 studio swing
    //   VP9: 01 4:2:0, 09 BT.2020, 16 PQ / 18 HLG, 09 BT.2020 NCL, 00 studio swing
    const colour = (tc: string) => codec === "AV1" ? `${base}.0.110.09.${tc}.09.0` : codec === "VP9" ? `${base}.01.09.${tc}.09.00` : base;
    const hdr = (tc: string, transferFunction: string, hdrMetadataType?: string) =>
      (codec === "H265" || supports(typeFor(mime, colour(tc))))
        ? decodes({ type, video: { contentType: typeFor(mime, colour(tc)), width: 3840, height: 2160, bitrate: 25_000_000, framerate: 24, transferFunction, colorGamut: "rec2020", ...(hdrMetadataType ? { hdrMetadataType } : {}) } })
          .then((r) => r?.supported ?? codec !== "H265")
        : Promise.resolve(false);
    const [pq, pqPlus, hlg] = await Promise.all([hdr("16", "pq", "smpteSt2086"), hdr("16", "pq", "smpteSt2094-40"), hdr("18", "hlg")]);
    if (pq) ranges.add("HDR10");
    if (pq && pqPlus) ranges.add("HDR10_PLUS");
    if (hlg) ranges.add("HLG");
  }
  // Dolby Vision decoders tone-map themselves, so no display check.
  const dv = (codecs: string) => supports(typeFor(mime === "video/webm" ? "video/mp4" : mime, codecs));
  if (codec === "H265" && tenBit) {
    const dl = doviLevel("H265", level);
    if (dv(`dvh1.05.${dl}`) || dv(`dvhe.05.${dl}`)) ranges.add("DOVI");
    if (dv(`dvh1.08.${dl}`) || dv(`dvhe.08.${dl}`)) ["DOVI_WITH_SDR", "DOVI_WITH_HLG", "DOVI_WITH_HDR10"].forEach((r) => ranges.add(r as VideoRange));
  } else if (codec === "AV1") {
    // Dolby Vision over AV1 is profile 10.
    if (dv(`dav1.10.${doviLevel("AV1", level)}`)) ["DOVI", "DOVI_WITH_SDR", "DOVI_WITH_HLG", "DOVI_WITH_HDR10"].forEach((r) => ranges.add(r as VideoRange));
  } else if (codec === "H264" && profile === "H264_HIGH") {
    // H.264 carries 8-bit Dolby Vision with an SDR-compatible base layer.
    if (dv(`dvav.09.${doviLevel("H264", level)}`)) ranges.add("DOVI_WITH_SDR");
  }
  return [...ranges];
}

async function codecSupport(codec: VideoCodec, supports: Probe, type: "media-source" | "file"): Promise<VideoCodecSupport | null> {
  const mime = mimeFor(codec);
  const ok = (codecs: string) => supports(typeFor(mime, codecs));
  const profiles = supportedProfiles(codec, ok);
  if (!profiles.length) return null;
  const top = profiles[0];
  // Walk levels at a depth the profile accepts: Firefox takes AV1 Professional at 12 bits only.
  const walkDepth = bitDepthCandidates(codec, top).findLast((depth) => ok(codecString(codec, top, undefined, depth)));
  const level = highestLevel(codec, top, walkDepth, ok);
  const bitDepth = highestBitDepth(codec, top, level, ok);
  const codecs = codecString(codec, top, level, bitDepth);
  const [resolution, ranges] = await Promise.all([
    maxResolution(mime, codecs, type),
    videoRanges(codec, top, level ?? levelTable(codec).at(-1) ?? 0, ok, type),
  ]);
  return {
    codec,
    profiles: profiles.filter((p): p is VideoProfile => !!p),
    maxLevel: level ?? null,
    maxBitDepth: bitDepth ?? null,
    maxWidth: resolution?.widest.width ?? null,
    maxHeight: resolution?.widest.height ?? null,
    maxPixels: maxPixels(resolution?.standard ?? null, levelMaxPixels(codec, level)),
    videoRanges: ranges,
  };
}

// MP3 in ISO-BMFF is object type 0x6B; browsers don't parse the
// `mp4a.40.34` spelling and log a warning for every probe that uses it.
const AUDIO: [AudioCodec, string][] = [
  ["AAC", "mp4a.40.2"], ["MP3", "mp4a.6B"], ["AC3", "ac-3"], ["EAC3", "ec-3"], ["OPUS", "opus"], ["FLAC", "fLaC"],
];

async function maxChannels(contentType: string): Promise<number> {
  for (const channels of ["8", "6"]) {
    const r = await decodes({ type: "file", audio: { contentType, channels, bitrate: 640_000, samplerate: 48_000 } });
    if (r?.supported) return Number(channels);
  }
  return 2;
}

async function probe(): Promise<ClientCapabilities> {
  const { canPlay, mse, nativeHls } = makeProbes();
  const either: Probe = (type) => canPlay(type) || !!mse?.(type);
  const mp4Video = (["H264", "H265", "AV1", "VP9"] as VideoCodec[]).filter((c) => canPlay(typeFor("video/mp4", codecString(c))));
  const webmVideo = (["VP8", "VP9", "AV1"] as VideoCodec[]).filter((c) => canPlay(typeFor("video/webm", codecString(c))));
  // E-AC-3 / AC-3 are platform decoders (Safari, Edge, some Chromes);
  // they pass only where the OS provides them.
  const mp4Audio = AUDIO.filter(([, s]) => canPlay(typeFor("audio/mp4", s))).map(([c]) => c);
  const webmAudio = (["OPUS", "VORBIS"] as AudioCodec[]).filter((c) => canPlay(typeFor("audio/webm", c.toLowerCase())));

  const directPlay: DirectPlayProfile[] = [];
  if (mp4Video.length || mp4Audio.length) directPlay.push({ container: "mp4", videoCodecs: mp4Video, audioCodecs: mp4Audio });
  if (webmVideo.length || webmAudio.length) directPlay.push({ container: "webm", videoCodecs: webmVideo, audioCodecs: webmAudio });
  const anyAudio = [...new Set([...mp4Audio, ...webmAudio])];
  // Chromium answers "" for Matroska MIME types yet demuxes Matroska for
  // `<video src>` with every codec it decodes elsewhere.
  const chromium = !!(navigator as any).userAgentData?.brands?.some((b: any) => b.brand === "Chromium");
  if (canPlay("video/x-matroska") || chromium) {
    const mkvVideo = [...new Set([...mp4Video, ...webmVideo])];
    directPlay.push({ container: "mkv", videoCodecs: mkvVideo, audioCodecs: anyAudio });
  }
  if (canPlay("audio/mpeg")) directPlay.push({ container: "mp3", videoCodecs: [], audioCodecs: ["MP3"] });
  if (canPlay("audio/flac")) directPlay.push({ container: "flac", videoCodecs: [], audioCodecs: ["FLAC"] });
  if (canPlay(typeFor("audio/ogg", "vorbis")) || canPlay(typeFor("audio/ogg", "opus"))) directPlay.push({ container: "ogg", videoCodecs: [], audioCodecs: ["VORBIS", "OPUS", "FLAC"].filter((c) => canPlay(typeFor("audio/ogg", c.toLowerCase()))) as AudioCodec[] });
  if (canPlay("audio/wav")) directPlay.push({ container: "wav", videoCodecs: [], audioCodecs: ["PCM"] });

  // HLS goes through hls.js wherever MSE exists, else native (old iOS).
  const hlsProbe: Probe | null = mse ?? (nativeHls ? canPlay : null);
  const hls: HlsProfile[] = [];
  if (hlsProbe) {
    const fmp4Video = (["H264", "H265", "AV1", "VP9"] as VideoCodec[]).filter((c) => hlsProbe(typeFor("video/mp4", codecString(c))));
    // Segments interleave audio with video, so MSE only ever sees the
    // audio inside a video/mp4 SourceBuffer; probe it there. Opus, the
    // ISO-BMFF MP3 types and Dolby fail this on most MSE stacks.
    const fmp4Audio = AUDIO.filter(([, s]) => hlsProbe(typeFor("video/mp4", `avc1.42E01E,${s}`))).map(([c]) => c);
    hls.push({ segmentContainer: "FMP4", videoCodecs: fmp4Video, audioCodecs: fmp4Audio });
    // TS only exists to keep MP3 stream-copyable; anything newer than
    // H.264 is better served by fMP4, and Dolby in TS was reported broken.
    const tsAudio: AudioCodec[] = fmp4Audio.includes("AAC") ? ["AAC"] : [];
    if (mse ? mse("audio/mpeg") : canPlay("audio/mpeg")) tsAudio.push("MP3");
    if (fmp4Video.includes("H264")) hls.push({ segmentContainer: "TS", videoCodecs: ["H264"], audioCodecs: tsAudio });
  }

  const videoCodecs = [...new Set([...mp4Video, ...webmVideo, ...hls.flatMap((h) => h.videoCodecs)])];
  const support = await Promise.all(videoCodecs.map((c) => codecSupport(c, either, mse ? "media-source" : "file")));
  const audioCodecs = await Promise.all(
    [...new Set([...anyAudio, ...hls.flatMap((h) => h.audioCodecs)])].map(async (codec) => {
      const s = AUDIO.find(([c]) => c === codec)?.[1];
      return { codec, maxChannels: s ? await maxChannels(typeFor("audio/mp4", s)) : 2 };
    }),
  );
  return {
    directPlay,
    hls,
    videoCodecs: support.filter((s): s is VideoCodecSupport => !!s),
    audioCodecs,
    subtitleCodecs: "track" in document.createElement("track") || "kind" in document.createElement("track") ? ["WEB_VTT"] : [],
    maxHeight: null,
    maxBitrateBps: null,
  };
}

let cached: Promise<ClientCapabilities> | null = null;

/**
 * This browser's capabilities, probed once per page load. A failed probe
 * is retried on the next call rather than cached.
 */
export function clientCapabilities(): Promise<ClientCapabilities> {
  cached ??= probe().catch((cause) => { cached = null; throw cause; });
  return cached;
}
