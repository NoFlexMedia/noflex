// SPDX-License-Identifier: AGPL-3.0-only
//
// What the app reads out of `MediaFile.probe`, the server's ffprobe report.

import '../utils/format.dart';

class VideoProbe {
  const VideoProbe({
    required this.codec,
    required this.width,
    required this.height,
    this.profile,
    this.level,
    this.bitDepth,
    this.frameRate,
    this.pixelFormat,
    this.transfer,
    this.doviProfile,
  });

  factory VideoProbe.fromJson(Map<String, dynamic> json) => VideoProbe(
    codec: (json['codec'] as String? ?? '').toLowerCase(),
    width: (json['width'] as num? ?? 0).toInt(),
    height: (json['height'] as num? ?? 0).toInt(),
    profile: json['profile'] as String?,
    level: json['level'] as num?,
    bitDepth: (json['bit_depth'] as num?)?.toInt(),
    frameRate: (json['frame_rate'] as num?)?.toDouble(),
    pixelFormat: json['pixel_format'] as String?,
    transfer: (json['transfer'] as String?)?.toLowerCase(),
    doviProfile: json['dovi_profile'],
  );

  final String codec;
  final int width;
  final int height;
  final String? profile;
  final num? level;
  final int? bitDepth;
  final double? frameRate;
  final String? pixelFormat;
  final String? transfer;
  final Object? doviProfile;

  int get pixels => width * height;
  bool get isHdr =>
      transfer == 'pq' || transfer == 'hlg' || doviProfile != null;

  /// What HDR this carries, as viewers name it; empty for SDR.
  String get dynamicRange => doviProfile != null
      ? 'Dolby Vision'
      : transfer == 'pq'
      ? 'HDR10'
      : transfer == 'hlg'
      ? 'HLG'
      : '';
}

class AudioProbe {
  const AudioProbe({
    required this.index,
    required this.codec,
    required this.channels,
    this.language,
    this.title,
    this.isDefault = false,
  });

  factory AudioProbe.fromJson(int index, Map<String, dynamic> json) =>
      AudioProbe(
        index: index,
        codec: json['codec'] as String? ?? '',
        channels: (json['channels'] as num? ?? 0).toInt(),
        language: json['language'] as String?,
        title: json['title'] as String?,
        isDefault: json['default'] as bool? ?? false,
      );

  /// Position among the file's audio streams, which `audioStream` takes.
  final int index;
  final String codec;
  final int channels;
  final String? language;
  final String? title;
  final bool isDefault;

  String get format => audioFormatName(codec, channels);
}

class MediaProbe {
  const MediaProbe({
    this.video,
    this.audio = const [],
    this.bitRate,
    this.durationMs,
  });

  factory MediaProbe.fromJson(Object? json) {
    if (json is! Map) return const MediaProbe();
    final map = json.cast<String, dynamic>();
    final video = map['video'];
    final audio = map['audio'];
    return MediaProbe(
      video: video is Map
          ? VideoProbe.fromJson(video.cast<String, dynamic>())
          : null,
      audio: audio is List
          ? [
              for (var i = 0; i < audio.length; i++)
                AudioProbe.fromJson(
                  i,
                  (audio[i] as Map).cast<String, dynamic>(),
                ),
            ]
          : const [],
      bitRate: (map['format_bit_rate'] as num?)?.toInt(),
      durationMs: (map['duration_ms'] as num?)?.toInt(),
    );
  }

  final VideoProbe? video;
  final List<AudioProbe> audio;
  final int? bitRate;
  final int? durationMs;

  /// "4K · HEVC · HDR10"; null without video.
  String? get versionLabel {
    final v = video;
    if (v == null || v.height == 0) return null;
    return [
      resolutionTier(v.height),
      codecName(v.codec),
      v.dynamicRange,
    ].where((part) => part.isNotEmpty).join(' · ');
  }

  List<String> techBadges(AudioProbe? chosenAudio) {
    final v = video;
    return [
      if (v != null && v.height > 0) resolutionTier(v.height),
      if (v != null) codecName(v.codec),
      if (v != null) v.dynamicRange,
      if (chosenAudio != null) chosenAudio.format,
    ].where((part) => part.isNotEmpty).toList();
  }
}
