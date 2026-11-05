// SPDX-License-Identifier: AGPL-3.0-only
//
// Which audio and subtitle tracks to start a title on: the viewer's preferred
// languages from their playback preferences, else what the file marks default.

import '../models/media_probe.dart';
import 'format.dart';

/// The `playback` preference, as Account → Profile writes it.
class PlaybackPrefs {
  const PlaybackPrefs({
    this.audioLanguages = const [],
    this.subtitlesOn = false,
    this.subtitleLanguages = const [],
  });

  factory PlaybackPrefs.fromJson(Object? value) {
    if (value is! Map) return const PlaybackPrefs();
    final audio = value['audio'];
    final subtitles = value['subtitles'];
    return PlaybackPrefs(
      audioLanguages: audio is Map
          ? List<String>.from(audio['languages'] as List? ?? const [])
          : const [],
      subtitlesOn: subtitles is Map && subtitles['enabled'] == true,
      subtitleLanguages: subtitles is Map
          ? List<String>.from(subtitles['languages'] as List? ?? const [])
          : const [],
    );
  }

  final List<String> audioLanguages;
  final bool subtitlesOn;
  final List<String> subtitleLanguages;

  Map<String, dynamic> toJson() => {
    'audio': {'languages': audioLanguages},
    'subtitles': {'enabled': subtitlesOn, 'languages': subtitleLanguages},
  };
}

/// A subtitle track, reduced to what choosing one needs.
class SubtitleChoice {
  const SubtitleChoice({
    required this.index,
    required this.codec,
    this.language,
    this.title,
    this.isDefault = false,
    this.forced = false,
    this.streamIndex,
    this.external = false,
  });

  /// Position in the file's `subtitleTracks`, which `burnSubtitle` takes.
  final int index;
  final String codec;
  final String? language;
  final String? title;
  final bool isDefault;
  final bool forced;

  /// Position among the container's subtitle streams, for muxed tracks.
  final int? streamIndex;
  final bool external;

  /// Image subtitles have no text to send.
  bool get isImage =>
      RegExp(r'pgs|dvd|dvb|vobsub|xsub', caseSensitive: false).hasMatch(codec);

  String get label =>
      title ??
      (languageName(language).isNotEmpty
          ? languageName(language)
          : 'Track ${index + 1}');
}

bool _speaks(String? trackLanguage, String language) =>
    trackLanguage != null &&
    languageCode(trackLanguage) == languageCode(language);

/// Index of the audio stream to start on; null without any to choose from.
int? preferredAudio(List<AudioProbe> streams, PlaybackPrefs prefs) {
  if (streams.isEmpty) return null;
  for (final language in prefs.audioLanguages) {
    final matching = streams.where((s) => _speaks(s.language, language));
    if (matching.isNotEmpty) {
      return matching
          .firstWhere((s) => s.isDefault, orElse: () => matching.first)
          .index;
    }
  }
  return streams
      .firstWhere((s) => s.isDefault, orElse: () => streams.first)
      .index;
}

/// Index of the subtitle track to start on; null for none.
int? preferredSubtitle(List<SubtitleChoice> tracks, PlaybackPrefs prefs) {
  if (tracks.isEmpty || !prefs.subtitlesOn) return null;
  for (final language in prefs.subtitleLanguages) {
    for (final track in tracks) {
      if (_speaks(track.language, language) && !track.forced) {
        return track.index;
      }
    }
  }
  if (prefs.subtitleLanguages.isNotEmpty) return null;
  for (final track in tracks) {
    if (track.isDefault && !track.forced) return track.index;
  }
  return null;
}
