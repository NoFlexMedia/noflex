// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:math' as math;

import 'package:intl/intl.dart';

String titleCase(String value) => value
    .toLowerCase()
    .replaceAll('_', ' ')
    .replaceAllMapped(RegExp(r'\b\w'), (m) => m[0]!.toUpperCase());

String runtime(int? milliseconds) {
  if (milliseconds == null || milliseconds == 0) return '';
  final minutes = (milliseconds / 60000).round();
  return minutes >= 60 ? '${minutes ~/ 60}h ${minutes % 60}m' : '${minutes}m';
}

String formatDate(DateTime? value, {bool time = false}) {
  if (value == null) return '—';
  final local = value.toLocal();
  return time
      ? DateFormat.yMMMd().add_jm().format(local)
      : DateFormat.yMMMd().format(local);
}

String monthYear(DateTime value) => DateFormat.yMMMM().format(value.toLocal());

/// How long ago, for sessions and history.
String ago(DateTime value, {DateTime? now}) {
  final delta = (now ?? DateTime.now()).difference(value);
  if (delta.inMinutes < 1) return 'just now';
  if (delta.inHours < 1) return '${delta.inMinutes}m ago';
  if (delta.inDays < 1) return '${delta.inHours}h ago';
  if (delta.inDays < 30) return '${delta.inDays}d ago';
  return formatDate(value);
}

String bytes(int? value) {
  if (value == null || value <= 0) return '—';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  final power = math.min(
    (math.log(value) / math.log(1024)).floor(),
    units.length - 1,
  );
  final scaled = value / math.pow(1024, power);
  return '${scaled.toStringAsFixed(power > 2 ? 1 : 0)} ${units[power]}';
}

/// A count at a glance: 1,234,567 is "1.2M".
String compactCount(int value) => NumberFormat.compact().format(value);

/// Player-style clock: "4:05", "1:02:03".
String timecode(int milliseconds) {
  final total = math.max(0, milliseconds ~/ 1000);
  final hours = total ~/ 3600;
  final minutes = (total ~/ 60) % 60;
  final seconds = (total % 60).toString().padLeft(2, '0');
  return hours > 0
      ? '$hours:${minutes.toString().padLeft(2, '0')}:$seconds'
      : '$minutes:$seconds';
}

const _codecNames = {
  'h264': 'H.264',
  'h265': 'HEVC',
  'hevc': 'HEVC',
  'av1': 'AV1',
  'vp9': 'VP9',
  'vp8': 'VP8',
  'mpeg2': 'MPEG-2',
  'mpeg2video': 'MPEG-2',
  'vc1': 'VC-1',
};

/// A codec enum or ffprobe name as viewers know it: `H265` → "HEVC".
String codecName(String? code) {
  final name = (code ?? '').toLowerCase();
  return _codecNames[name] ?? name.toUpperCase();
}

const _audioNames = {
  'aac': 'AAC',
  'ac3': 'AC-3',
  'eac3': 'E-AC-3',
  'truehd': 'TrueHD',
  'dts': 'DTS',
  'flac': 'FLAC',
  'opus': 'Opus',
  'vorbis': 'Vorbis',
  'mp3': 'MP3',
  'pcm_s16le': 'PCM',
  'pcm_s24le': 'PCM',
};
const _channelNames = {1: 'Mono', 2: 'Stereo', 6: '5.1', 8: '7.1'};

String audioFormatName(String codec, int? channels) {
  final lower = codec.toLowerCase();
  return [
    _audioNames[lower] ?? lower.toUpperCase(),
    _channelNames[channels] ??
        (channels != null && channels > 0 ? '$channels ch' : ''),
  ].where((part) => part.isNotEmpty).join(' ');
}

String resolutionTier(int height) => height >= 2000
    ? '4K'
    : height >= 1400
    ? '1440p'
    : height >= 1000
    ? '1080p'
    : height >= 700
    ? '720p'
    : '${height}p';

/// A level in the server's (ffprobe's) units: H.264 ×10, HEVC ×30, AV1 `seq_level_idx`.
String? levelName(String? codec, num? level) {
  if (level == null || level <= 0) return null;
  final name = (codec ?? '').toLowerCase();
  if (name == 'h264') return 'L${level / 10}';
  if (name == 'h265' || name == 'hevc') return 'L${(level / 3).round() / 10}';
  return 'L$level';
}

/// A profile as usually written: `HEVC_MAIN10` → "Main 10".
String profileName(String? profile) {
  if (profile == null || profile.isEmpty) return '';
  final spaced = profile
      .replaceFirst(RegExp(r'^(H264|HEVC|AV1|VP9)_', caseSensitive: false), '')
      .replaceAllMapped(RegExp(r'(\D)(\d+)$'), (m) => '${m[1]} ${m[2]}')
      .replaceAll('_', ' ')
      .toLowerCase();
  return spaced.isEmpty
      ? ''
      : '${spaced[0].toUpperCase()}${spaced.substring(1)}';
}

const _iso6392 = {
  'eng': 'en',
  'spa': 'es',
  'fre': 'fr',
  'fra': 'fr',
  'ger': 'de',
  'deu': 'de',
  'ita': 'it',
  'jpn': 'ja',
  'kor': 'ko',
  'chi': 'zh',
  'zho': 'zh',
  'por': 'pt',
  'rus': 'ru',
  'dut': 'nl',
  'nld': 'nl',
  'swe': 'sv',
  'nor': 'no',
  'dan': 'da',
  'fin': 'fi',
  'pol': 'pl',
  'tur': 'tr',
  'ara': 'ar',
  'hin': 'hi',
};
const _languages = {
  'en': 'English',
  'es': 'Spanish',
  'fr': 'French',
  'de': 'German',
  'it': 'Italian',
  'ja': 'Japanese',
  'ko': 'Korean',
  'zh': 'Chinese',
  'pt': 'Portuguese',
  'ru': 'Russian',
  'nl': 'Dutch',
  'sv': 'Swedish',
  'no': 'Norwegian',
  'da': 'Danish',
  'fi': 'Finnish',
  'pl': 'Polish',
  'tr': 'Turkish',
  'ar': 'Arabic',
  'hi': 'Hindi',
};

/// A language tag in the one spelling used to compare them: `eng` and `EN` → `en`.
String languageCode(String code) {
  final lower = code.trim().toLowerCase();
  return _iso6392[lower] ?? lower;
}

String languageName(String? code) {
  if (code == null || code.isEmpty) return '';
  return _languages[languageCode(code)] ?? code.toUpperCase();
}
