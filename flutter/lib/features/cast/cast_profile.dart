// SPDX-License-Identifier: AGPL-3.0-only
//
// What a Cast device decodes when it can't say: Google's Default Media
// Receiver, which answers nothing, goes by its model name. A port of
// web/src/lib/castProfile.ts; the table changes in both. NoFlex's receiver
// measures itself instead (`capabilities`, cast/AGENTS.md).

import '../../graphql/__generated__/schema.schema.gql.dart';

const _audio = [
  {'codec': 'AAC', 'maxChannels': 6},
  {'codec': 'AC3', 'maxChannels': 6},
  {'codec': 'EAC3', 'maxChannels': 6},
  {'codec': 'MP3', 'maxChannels': 2},
  {'codec': 'OPUS', 'maxChannels': 2},
  {'codec': 'FLAC', 'maxChannels': 2},
  {'codec': 'VORBIS', 'maxChannels': 2},
];

Map<String, Object> _h264(int level, int width, int height) => {
  'codec': 'H264',
  'profiles': ['H264_HIGH', 'H264_MAIN', 'H264_CONSTRAINED_BASELINE'],
  'maxLevel': level,
  'maxBitDepth': 8,
  'maxWidth': width,
  'maxHeight': height,
  'maxPixels': width * height,
  'videoRanges': ['SDR'],
};

Map<String, Object> _hevc(List<String> ranges) => {
  'codec': 'H265',
  'profiles': ['HEVC_MAIN10', 'HEVC_MAIN'],
  'maxLevel': 153,
  'maxBitDepth': 10,
  'maxWidth': 3840,
  'maxHeight': 2160,
  'maxPixels': 3840 * 2160,
  'videoRanges': ranges,
};

Map<String, Object> _vp9(
  int level,
  int width,
  int height,
  List<String> ranges,
) => {
  'codec': 'VP9',
  'profiles': ranges.length > 1
      ? ['VP9_PROFILE2', 'VP9_PROFILE0']
      : ['VP9_PROFILE0'],
  'maxLevel': level,
  'maxBitDepth': ranges.length > 1 ? 10 : 8,
  'maxWidth': width,
  'maxHeight': height,
  'maxPixels': width * height,
  'videoRanges': ranges,
};

const _av1 = {
  'codec': 'AV1',
  'profiles': ['AV1_MAIN'],
  'maxLevel': 13,
  'maxBitDepth': 10,
  'maxWidth': 3840,
  'maxHeight': 2160,
  'maxPixels': 3840 * 2160,
  'videoRanges': ['SDR', 'HDR10'],
};

const _uhdHdr = ['SDR', 'HDR10', 'HLG'];

List<Map<String, Object>> _videoFor(String model) {
  bool has(String pattern) =>
      RegExp(pattern, caseSensitive: false).hasMatch(model);
  if (has('streamer')) {
    return [
      _h264(52, 3840, 2160),
      _hevc([..._uhdHdr, 'DOVI', 'DOVI_WITH_HDR10']),
      _vp9(51, 3840, 2160, _uhdHdr),
      _av1,
    ];
  }
  if (has('ultra')) {
    return [
      _h264(42, 1920, 1080),
      _hevc(['SDR', 'HDR10', 'DOVI', 'DOVI_WITH_HDR10']),
      _vp9(51, 3840, 2160, ['SDR', 'HDR10']),
    ];
  }
  if (has('google tv')) {
    return [
      _h264(51, 3840, 2160),
      _hevc(_uhdHdr),
      _vp9(51, 3840, 2160, _uhdHdr),
    ];
  }
  if (has('nest hub')) {
    return [
      _h264(41, 1280, 720),
      _vp9(40, 1280, 720, ['SDR']),
    ];
  }
  // A TV with Cast built in (Android TV, Google TV) reports its own model. A 4K
  // TV decodes HEVC and VP9 to stream at all; HDR is left to the probe.
  if (!has('chromecast') &&
      has(r'\b(4k|uhd|2160p|oled|qled)\b|bravia|android ?tv')) {
    return [
      _h264(51, 3840, 2160),
      _hevc(['SDR']),
      _vp9(51, 3840, 2160, ['SDR']),
    ];
  }
  // The first Chromecasts stop at level 4.1, so a 1080p60 stream is beyond them.
  return [_h264(41, 1920, 1080)];
}

/// The profile to ask the server for when [model] is the Cast device that
/// will play and it can't be asked.
GClientCapabilities castCapabilities(String model) {
  final video = _videoFor(model);
  final codecs = [for (final item in video) item['codec']! as String];
  const inMp4 = ['AAC', 'AC3', 'EAC3', 'MP3'];
  final direct = [
    {
      'container': 'mp4',
      'videoCodecs': [
        for (final c in codecs)
          if (c == 'H264' || c == 'H265') c,
      ],
      'audioCodecs': inMp4,
    },
    {
      'container': 'webm',
      'videoCodecs': [
        for (final c in codecs)
          if (c == 'VP9' || c == 'AV1') c,
      ],
      'audioCodecs': ['OPUS', 'VORBIS'],
    },
  ];
  return GClientCapabilities.fromJson({
    'directPlay': [
      for (final profile in direct)
        if ((profile['videoCodecs']! as List).isNotEmpty) profile,
    ],
    'hls': [
      {
        'segmentContainer': 'FMP4',
        'videoCodecs': codecs,
        'audioCodecs': [for (final a in _audio) a['codec']],
      },
      {
        'segmentContainer': 'TS',
        'videoCodecs': [
          for (final c in codecs)
            if (c == 'H264') c,
        ],
        'audioCodecs': inMp4,
      },
    ],
    'videoCodecs': video,
    'audioCodecs': _audio,
    'subtitleCodecs': ['WEB_VTT'],
    'maxHeight': video
        .map((item) => item['maxHeight']! as int)
        .reduce((a, b) => a > b ? a : b),
    'maxBitrateBps': null,
  })!;
}
