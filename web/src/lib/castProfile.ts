// SPDX-License-Identifier: AGPL-3.0-only
//
// What a Cast device decodes, as the server's `ClientCapabilities`. NoFlex's
// receiver answers for itself: the sender runs the web client's own probe
// (`probeElsewhere`) with each question put to the device's `canDisplayType`,
// so a TV with Cast built in or a new Chromecast is measured, not guessed.
// Google's Default Media Receiver can't be asked anything, so for it this
// goes by the model name: a safe H.264 profile for anything unknown, and more
// for the devices Google lists (developers.google.com/cast/docs/media).

import { probeElsewhere, type AudioCodec, type AudioCodecSupport, type ClientCapabilities, type DirectPlayProfile, type MediaQuestion, type VideoCodecSupport } from "$codecs";

const AUDIO: AudioCodecSupport[] = [
  { codec: "AAC", maxChannels: 6 }, { codec: "AC3", maxChannels: 6 }, { codec: "EAC3", maxChannels: 6 },
  { codec: "MP3", maxChannels: 2 }, { codec: "OPUS", maxChannels: 2 }, { codec: "FLAC", maxChannels: 2 }, { codec: "VORBIS", maxChannels: 2 },
];

const h264 = (level: number, width: number, height: number): VideoCodecSupport => ({
  codec: "H264", profiles: ["H264_HIGH", "H264_MAIN", "H264_CONSTRAINED_BASELINE"], maxLevel: level, maxBitDepth: 8,
  maxWidth: width, maxHeight: height, maxPixels: width * height, videoRanges: ["SDR"],
});
const hevc = (ranges: VideoCodecSupport["videoRanges"]): VideoCodecSupport => ({
  codec: "H265", profiles: ["HEVC_MAIN10", "HEVC_MAIN"], maxLevel: 153, maxBitDepth: 10,
  maxWidth: 3840, maxHeight: 2160, maxPixels: 3840 * 2160, videoRanges: ranges,
});
const vp9 = (level: number, width: number, height: number, ranges: VideoCodecSupport["videoRanges"]): VideoCodecSupport => ({
  codec: "VP9", profiles: ranges.length > 1 ? ["VP9_PROFILE2", "VP9_PROFILE0"] : ["VP9_PROFILE0"], maxLevel: level, maxBitDepth: ranges.length > 1 ? 10 : 8,
  maxWidth: width, maxHeight: height, maxPixels: width * height, videoRanges: ranges,
});
const av1: VideoCodecSupport = {
  codec: "AV1", profiles: ["AV1_MAIN"], maxLevel: 13, maxBitDepth: 10,
  maxWidth: 3840, maxHeight: 2160, maxPixels: 3840 * 2160, videoRanges: ["SDR", "HDR10"],
};
const UHD_HDR: VideoCodecSupport["videoRanges"] = ["SDR", "HDR10", "HLG"];

function videoFor(model: string): VideoCodecSupport[] {
  if (/streamer/i.test(model)) return [h264(52, 3840, 2160), hevc([...UHD_HDR, "DOVI", "DOVI_WITH_HDR10"]), vp9(51, 3840, 2160, UHD_HDR), av1];
  if (/ultra/i.test(model)) return [h264(42, 1920, 1080), hevc(["SDR", "HDR10", "DOVI", "DOVI_WITH_HDR10"]), vp9(51, 3840, 2160, ["SDR", "HDR10"])];
  if (/google tv/i.test(model)) return [h264(51, 3840, 2160), hevc(UHD_HDR), vp9(51, 3840, 2160, UHD_HDR)];
  if (/nest hub/i.test(model)) return [h264(41, 1280, 720), vp9(40, 1280, 720, ["SDR"])];
  // The first Chromecasts stop at level 4.1, so a 1080p60 stream is beyond them.
  return [h264(41, 1920, 1080)];
}

/** The profile to ask the server for when `model` is the Cast device that will play and it can't be asked. */
export function castCapabilities(model: string): ClientCapabilities {
  const video = videoFor(model);
  const codecs = video.map((item) => item.codec);
  const audio = AUDIO.map((item) => item.codec);
  const inMp4: AudioCodec[] = ["AAC", "AC3", "EAC3", "MP3"];
  const direct: DirectPlayProfile[] = [
    { container: "mp4", videoCodecs: codecs.filter((codec) => codec === "H264" || codec === "H265"), audioCodecs: inMp4 },
    { container: "webm", videoCodecs: codecs.filter((codec) => codec === "VP9" || codec === "AV1"), audioCodecs: ["OPUS", "VORBIS"] },
  ];
  return {
    directPlay: direct.filter((profile) => profile.videoCodecs.length),
    hls: [
      { segmentContainer: "FMP4", videoCodecs: codecs, audioCodecs: audio },
      { segmentContainer: "TS", videoCodecs: codecs.filter((codec) => codec === "H264"), audioCodecs: inMp4 },
    ],
    videoCodecs: video,
    audioCodecs: AUDIO,
    subtitleCodecs: ["WEB_VTT"],
    maxHeight: Math.max(...video.map((item) => item.maxHeight ?? 0)),
    maxBitrateBps: null,
  };
}

/** A receiver's answer to a `device` message. */
export interface CastDevice { hdr: boolean; dolbyVision: boolean; touch: boolean }

/** What NoFlex's receiver says it decodes, asked through `ask`. */
export function probeCast(device: CastDevice, ask: (questions: MediaQuestion[]) => Promise<boolean[]>): Promise<ClientCapabilities> {
  return probeElsewhere(ask, device.hdr);
}
