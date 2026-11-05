// SPDX-License-Identifier: AGPL-3.0-only

import '../../models/work.dart';

enum OfflineStatus { queued, downloading, paused, complete, failed }

/// Who decided this title should be on the device. Only smart ones are the
/// server's to take back; what the viewer chose stays until they remove it.
enum OfflineOrigin { smart, manual }

class OfflineSubtitle {
  const OfflineSubtitle({
    required this.index,
    required this.fileName,
    this.language,
    this.title,
    this.forced = false,
    this.isDefault = false,
  });

  factory OfflineSubtitle.fromJson(Map<String, dynamic> json) =>
      OfflineSubtitle(
        index: json['index'] as int,
        fileName: json['fileName'] as String,
        language: json['language'] as String?,
        title: json['title'] as String?,
        forced: json['forced'] as bool? ?? false,
        isDefault: json['default'] as bool? ?? false,
      );

  final int index;
  final String fileName;
  final String? language;
  final String? title;
  final bool forced;
  final bool isDefault;

  Map<String, dynamic> toJson() => {
    'index': index,
    'fileName': fileName,
    'language': language,
    'title': title,
    'forced': forced,
    'default': isDefault,
  };
}

/// One file held, or being fetched, for offline. Parts of a multi-file title are
/// separate entries.
class OfflineItem {
  const OfflineItem({
    required this.work,
    required this.partIndex,
    required this.origin,
    required this.status,
    required this.sizeBytes,
    required this.fileName,
    required this.addedAt,
    this.directory,
    this.progress = 0,
    this.optimized = false,
    this.subtitles = const [],
    this.posterFile,
    this.positionMs = 0,
    this.played = false,
    this.error,
  });

  factory OfflineItem.fromJson(Map<String, dynamic> json) => OfflineItem(
    work: WorkSummary.fromJson((json['work'] as Map).cast<String, dynamic>()),
    partIndex: json['partIndex'] as int,
    origin: OfflineOrigin.values.byName(json['origin'] as String),
    status: OfflineStatus.values.byName(json['status'] as String),
    sizeBytes: json['sizeBytes'] as int,
    fileName: json['fileName'] as String,
    addedAt: DateTime.parse(json['addedAt'] as String),
    directory: json['directory'] as String?,
    progress: (json['progress'] as num?)?.toDouble() ?? 0,
    optimized: json['optimized'] as bool? ?? false,
    subtitles: [
      for (final subtitle in json['subtitles'] as List? ?? const [])
        OfflineSubtitle.fromJson((subtitle as Map).cast<String, dynamic>()),
    ],
    posterFile: json['posterFile'] as String?,
    positionMs: json['positionMs'] as int? ?? 0,
    played: json['played'] as bool? ?? false,
    error: json['error'] as String?,
  );

  final WorkSummary work;
  final int partIndex;
  final OfflineOrigin origin;
  final OfflineStatus status;
  final int sizeBytes;
  final String fileName;
  final DateTime addedAt;

  /// An absolute folder the viewer chose; null for the app's own storage, whose
  /// absolute path moves between launches on iOS and so is never stored.
  final String? directory;
  final double progress;
  final bool optimized;
  final List<OfflineSubtitle> subtitles;
  final String? posterFile;
  final int positionMs;
  final bool played;
  final String? error;

  String get key => '${work.id}-$partIndex';

  /// The background downloader's id for this file's transfer.
  String get taskId => key;

  bool get isFinished => status == OfflineStatus.complete;

  OfflineItem copyWith({
    OfflineOrigin? origin,
    OfflineStatus? status,
    double? progress,
    int? positionMs,
    bool? played,
    String? error,
    bool clearError = false,
    String? posterFile,
    List<OfflineSubtitle>? subtitles,
  }) => OfflineItem(
    work: work,
    partIndex: partIndex,
    origin: origin ?? this.origin,
    status: status ?? this.status,
    sizeBytes: sizeBytes,
    fileName: fileName,
    addedAt: addedAt,
    directory: directory,
    progress: progress ?? this.progress,
    optimized: optimized,
    subtitles: subtitles ?? this.subtitles,
    posterFile: posterFile ?? this.posterFile,
    positionMs: positionMs ?? this.positionMs,
    played: played ?? this.played,
    error: clearError ? null : (error ?? this.error),
  );

  Map<String, dynamic> toJson() => {
    'work': work.toJson(),
    'partIndex': partIndex,
    'origin': origin.name,
    'status': status.name,
    'sizeBytes': sizeBytes,
    'fileName': fileName,
    'addedAt': addedAt.toIso8601String(),
    'directory': directory,
    'progress': progress,
    'optimized': optimized,
    'subtitles': [for (final subtitle in subtitles) subtitle.toJson()],
    'posterFile': posterFile,
    'positionMs': positionMs,
    'played': played,
    'error': error,
  };
}

/// A position watched without the server, carried over once it's back.
class PendingReport {
  const PendingReport({
    required this.workId,
    required this.positionMs,
    this.played = false,
  });

  factory PendingReport.fromJson(Map<String, dynamic> json) => PendingReport(
    workId: json['workId'] as String,
    positionMs: json['positionMs'] as int,
    played: json['played'] as bool? ?? false,
  );

  final String workId;
  final int positionMs;
  final bool played;

  Map<String, dynamic> toJson() => {
    'workId': workId,
    'positionMs': positionMs,
    'played': played,
  };
}

/// A file the server wants this device to hold.
class WantedFile {
  const WantedFile({
    required this.work,
    required this.partIndex,
    required this.url,
    required this.sizeBytes,
    this.optimized = false,
    this.subtitles = const [],
  });

  final WorkSummary work;
  final int partIndex;
  final String url;
  final int sizeBytes;
  final bool optimized;
  final List<WantedSubtitle> subtitles;

  String get key => '${work.id}-$partIndex';
}

class WantedSubtitle {
  const WantedSubtitle({
    required this.index,
    required this.url,
    this.language,
    this.title,
    this.forced = false,
    this.isDefault = false,
  });

  final int index;
  final String url;
  final String? language;
  final String? title;
  final bool forced;
  final bool isDefault;
}
