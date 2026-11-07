// SPDX-License-Identifier: AGPL-3.0-only
//
// What a Cast device decodes, as the server's `ClientCapabilities`. The sender
// can't probe a receiver as a browser probes itself, so this goes by the device's
// model: a safe H.264 profile for anything unknown (a TV with Cast built in
// reports its own model name), and more for the Chromecasts known to do more.
// Numbers are Google's published device table (developers.google.com/cast/docs/media).

import type { AudioCodec, AudioCodecSupport, ClientCapabilities, DirectPlayProfile, VideoCodecSupport } from "$codecs";

const AUDIO: AudioCodecSupport[] = [
  { codec: "AAC", maxChannels: 6 }, { codec: "AC3", maxChannels: 6 }, { codec: "EAC3", maxChannels: 6 },
  { codec: "MP3", maxChannels: 2 }, { codec: "OPUS", maxChannels: 2 }, { codec: "FLAC", maxChannels: 2 }, { codec: "VORBIS", maxChannels: 2 },
];

const h264 = (level: number, width: number, height: number): VideoCodecSupport => ({
  codec: "H264", profiles: ["H264_HIGH", "H264_MAIN", "H264_CONSTRAINED_BASELINE"], maxLevel: level, maxBitDepth: 8,
  maxWidth: width, maxHeight: height, maxPixels: width * height, videoRanges: ["SDR"],
});
const hevc = (ranges: VideoCodecSupport["videoRanges"]): VideoCodecSupport => ({
  codec: "H265", profiles: ["HEVC_MAIN", "HEVC_MAIN10"], maxLevel: 153, maxBitDepth: 10,
  maxWidth: 3840, maxHeight: 2160, maxPixels: 3840 * 2160, videoRanges: ranges,
});
const vp9: VideoCodecSupport = {
  codec: "VP9", profiles: ["VP9_PROFILE0", "VP9_PROFILE2"], maxLevel: 51, maxBitDepth: 10,
  maxWidth: 3840, maxHeight: 2160, maxPixels: 3840 * 2160, videoRanges: ["SDR", "HDR10"],
};
const av1: VideoCodecSupport = {
  codec: "AV1", profiles: ["AV1_MAIN"], maxLevel: 13, maxBitDepth: 10,
  maxWidth: 3840, maxHeight: 2160, maxPixels: 3840 * 2160, videoRanges: ["SDR", "HDR10"],
};

function videoFor(model: string): VideoCodecSupport[] {
  if (/streamer/i.test(model)) return [h264(52, 3840, 2160), hevc(["SDR", "HDR10", "DOVI"]), vp9, av1];
  if (/ultra/i.test(model)) return [h264(42, 3840, 2160), hevc(["SDR", "HDR10", "DOVI"]), vp9];
  if (/google tv/i.test(model)) return [h264(51, 3840, 2160), hevc(["SDR", "HDR10"]), vp9];
  return [h264(42, 1920, 1080)];
}

/** The profile to ask the server for when `model` is the Cast device that will play. */
export function castCapabilities(model: string): ClientCapabilities {
  const video = videoFor(model);
  const codecs = video.map((item) => item.codec);
  const audio = AUDIO.map((item) => item.codec);
  const inMp4: AudioCodec[] = ["AAC", "AC3", "EAC3", "MP3"];
  const direct: DirectPlayProfile[] = [
    { container: "mp4", videoCodecs: codecs.filter((codec) => codec === "H264" || codec === "H265"), audioCodecs: inMp4 },
    { container: "webm", videoCodecs: codecs.filter((codec) => codec === "VP9" || codec === "AV1"), audioCodecs: ["OPUS", "VORBIS"] },
  ];
  const tallest = Math.max(...video.map((item) => item.maxHeight ?? 0));
  return {
    directPlay: direct.filter((profile) => profile.videoCodecs.length),
    hls: [
      { segmentContainer: "FMP4", videoCodecs: codecs, audioCodecs: audio },
      { segmentContainer: "TS", videoCodecs: codecs.filter((codec) => codec === "H264"), audioCodecs: inMp4 },
    ],
    videoCodecs: video,
    audioCodecs: AUDIO,
    subtitleCodecs: ["WEB_VTT"],
    maxHeight: tallest,
    maxBitrateBps: null,
  };
}
