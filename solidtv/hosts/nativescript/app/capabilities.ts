// SPDX-License-Identifier: AGPL-3.0-only
//
// What this Apple device can decode, measured rather than assumed, through the
// same codec strings and walks as the browsers' probe. AVFoundation answers for
// the types and VideoToolbox for whether hardware does it: on an Apple TV HD, HEVC
// would be software, so it's left for the server to transcode.

import { codecString, highestBitDepth, highestLevel, supportedProfiles, type AudioCodec, type ClientCapabilities, type VideoCodec, type VideoCodecSupport, type VideoRange } from "$codecs";

declare function VTIsHardwareDecodeSupported(codecType: number): boolean;

// FourCCs: 'avc1', 'hvc1', 'av01'.
const FOURCC: Partial<Record<VideoCodec, number>> = { H264: 0x61766331, H265: 0x68766331, AV1: 0x61763031 };
// AVPlayerHDRMode.
const HDR_HLG = 1;
const HDR_10 = 2;
const HDR_DOLBY_VISION = 4;

const playable = (type: string): boolean => AVURLAsset.isPlayableExtendedMIMEType(type);
// Without the answer, trust AVFoundation's: it plays what it says, if less efficiently.
const hardware = (codec: VideoCodec): boolean => {
  const code = FOURCC[codec];
  return code != null && (typeof VTIsHardwareDecodeSupported !== "function" || VTIsHardwareDecodeSupported(code));
};

const AUDIO: [AudioCodec, string, number][] = [["AAC", "mp4a.40.2", 8], ["AC3", "ac-3", 6], ["EAC3", "ec-3", 8], ["FLAC", "fLaC", 8], ["MP3", "mp4a.6B", 2], ["OPUS", "opus", 2]];

function hdrRanges(): VideoRange[] {
  const ranges: VideoRange[] = [];
  if (!AVPlayer.eligibleForHDRPlayback) return ranges;
  const modes = AVPlayer.availableHDRModes;
  if (modes & HDR_10) ranges.push("HDR10");
  if (modes & HDR_HLG) ranges.push("HLG");
  if (modes & HDR_DOLBY_VISION) ranges.push("DOVI", "DOVI_WITH_HDR10", "DOVI_WITH_SDR", "DOVI_WITH_HLG");
  return ranges;
}

function videoSupport(codec: VideoCodec, width: number, height: number): VideoCodecSupport | null {
  if (!hardware(codec)) return null;
  const supports = (codecs: string) => playable(`video/mp4; codecs="${codecs}"`);
  const profiles = supportedProfiles(codec, supports).filter((profile): profile is NonNullable<typeof profile> => !!profile);
  if (!profiles.length) return null;
  const top = profiles[0];
  const level = highestLevel(codec, top, undefined, supports);
  const bitDepth = highestBitDepth(codec, top, level, supports);
  const tenBit = (bitDepth ?? 8) >= 10;
  return {
    codec,
    profiles,
    maxLevel: level ?? null,
    maxBitDepth: bitDepth ?? null,
    maxWidth: width,
    maxHeight: height,
    maxPixels: width * height,
    videoRanges: ["SDR", ...(tenBit ? hdrRanges() : [])],
  };
}

let cached: ClientCapabilities | null = null;

export function appleCapabilities(): Promise<ClientCapabilities> {
  if (cached) return Promise.resolve(cached);
  const native = UIScreen.mainScreen.nativeBounds.size;
  const width = Math.max(native.width, native.height);
  const height = Math.min(native.width, native.height);
  const video = (["H264", "H265", "AV1"] as VideoCodec[]).map((codec) => videoSupport(codec, width, height)).filter((item): item is VideoCodecSupport => !!item);
  const audio = AUDIO.filter(([, codecs]) => playable(`audio/mp4; codecs="${codecs}"`));
  const videoCodecs = video.map((item) => item.codec);
  const audioCodecs = audio.map(([codec]) => codec);
  // AVPlayer takes a file in an MP4 or QuickTime container, and HLS for everything else.
  cached = {
    directPlay: [
      { container: "mp4", videoCodecs, audioCodecs },
      { container: "mov", videoCodecs, audioCodecs },
    ],
    hls: [
      { segmentContainer: "FMP4", videoCodecs, audioCodecs },
      { segmentContainer: "TS", videoCodecs: ["H264"], audioCodecs: audioCodecs.filter((codec) => codec === "AAC" || codec === "MP3" || codec === "AC3" || codec === "EAC3") },
    ],
    videoCodecs: video,
    audioCodecs: audio.map(([codec, , maxChannels]) => ({ codec, maxChannels })),
    subtitleCodecs: ["WEB_VTT"],
    maxHeight: null,
    maxBitrateBps: null,
  };
  return Promise.resolve(cached);
}
