// SPDX-License-Identifier: AGPL-3.0-only
//
// This player in the shape of the server's `ClientCapabilities`. libmpv demuxes
// and decodes nearly everything in software, so the limits here are about what
// keeps up: the better of measured hardware and the software ceiling.

import 'package:built_collection/built_collection.dart';

import '../../graphql/__generated__/schema.schema.gql.dart';
import 'decoders.dart';

const _videoCodecs = {
  'h264': GVideoCodec.H264,
  'h265': GVideoCodec.H265,
  'av1': GVideoCodec.AV1,
  'vp9': GVideoCodec.VP9,
};

/// libmpv's decoder names per server codec.
const _audioCodecs = {
  GAudioCodec.AAC: 'aac',
  GAudioCodec.AC3: 'ac3',
  GAudioCodec.EAC3: 'eac3',
  GAudioCodec.MP3: 'mp3',
  GAudioCodec.OPUS: 'opus',
  GAudioCodec.VORBIS: 'vorbis',
  GAudioCodec.FLAC: 'flac',
  GAudioCodec.TRUEHD: 'truehd',
  GAudioCodec.DTS: 'dts',
  GAudioCodec.PCM: 'pcm_s16le',
};

/// libmpv tone-maps these itself. Dolby Vision profile 5 is left out: without a
/// compatible base layer it needs libplacebo's reshaping, which the video
/// outputs media_kit uses don't do, and it would come out purple.
final _ranges = BuiltList<GVideoRange>([
  GVideoRange.HDR10,
  GVideoRange.HDR10_PLUS,
  GVideoRange.HLG,
  GVideoRange.DOVI_WITH_HDR10,
  GVideoRange.DOVI_WITH_SDR,
  GVideoRange.DOVI_WITH_HLG,
]);

GClientCapabilities buildCapabilities(
  DecoderProfile profile, {
  int? maxBitrateBps,
}) {
  final audio = [
    for (final entry in _audioCodecs.entries)
      if (profile.audioDecoders.isEmpty ||
          profile.audioDecoders.contains(entry.value))
        entry.key,
  ];
  final video = [
    ..._videoCodecs.values,
    GVideoCodec.VP8,
    GVideoCodec.MPEG2,
    GVideoCodec.THEORA,
  ];

  GDirectPlayProfile direct(
    String container,
    List<GVideoCodec> v,
    List<GAudioCodec> a,
  ) => GDirectPlayProfile(
    (b) => b
      ..container = container
      ..videoCodecs.replace(v)
      ..audioCodecs.replace(a),
  );

  return GClientCapabilities(
    (b) => b
      ..directPlay.replace([
        direct('mkv', video, audio),
        direct('mp4', video, audio),
        direct('webm', video, audio),
        direct('ts', video, audio),
        direct('mp3', const [], [GAudioCodec.MP3]),
        direct('flac', const [], [GAudioCodec.FLAC]),
        direct('ogg', const [], [
          GAudioCodec.VORBIS,
          GAudioCodec.OPUS,
          GAudioCodec.FLAC,
        ]),
        direct('wav', const [], [GAudioCodec.PCM]),
      ])
      ..hls.replace([
        GHlsProfile(
          (h) => h
            ..segmentContainer = GHlsSegmentContainer.FMP4
            ..videoCodecs.replace(_videoCodecs.values)
            ..audioCodecs.replace([
              for (final codec in audio)
                if (codec != GAudioCodec.TRUEHD &&
                    codec != GAudioCodec.DTS &&
                    codec != GAudioCodec.PCM)
                  codec,
            ]),
        ),
        GHlsProfile(
          (h) => h
            ..segmentContainer = GHlsSegmentContainer.TS
            ..videoCodecs.replace([GVideoCodec.H264])
            ..audioCodecs.replace([
              GAudioCodec.AAC,
              GAudioCodec.MP3,
              GAudioCodec.AC3,
            ]),
        ),
      ])
      ..videoCodecs.replace([
        for (final entry in _videoCodecs.entries)
          _support(entry.key, entry.value, profile),
      ])
      ..audioCodecs.replace([
        for (final codec in audio)
          GAudioCodecSupport(
            (a) => a
              ..codec = codec
              ..maxChannels = 8,
          ),
      ])
      ..subtitleCodecs.replace([
        GSubtitleCodec.WEB_VTT,
        GSubtitleCodec.SRT,
        GSubtitleCodec.SUB_RIP,
        GSubtitleCodec.ASS,
        GSubtitleCodec.SSA,
      ])
      ..maxBitrateBps = maxBitrateBps,
  );
}

GVideoCodecSupport _support(
  String name,
  GVideoCodec codec,
  DecoderProfile profile,
) {
  final software = profile.softwareCeiling;
  final hardware = profile.hardware[name] ?? const <DecodeCeiling>[];
  var pixels = software.pixels;
  var depth = software.bitDepth;
  for (final ceiling in hardware) {
    if (ceiling.pixels > pixels) pixels = ceiling.pixels;
    if (ceiling.bitDepth > depth) depth = ceiling.bitDepth;
  }
  return GVideoCodecSupport(
    (b) => b
      ..codec = codec
      ..maxBitDepth = depth
      ..maxPixels = pixels
      ..videoRanges.replace(depth >= 10 ? _ranges : const <GVideoRange>[]),
  );
}
