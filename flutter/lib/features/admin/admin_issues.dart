// SPDX-License-Identifier: AGPL-3.0-only
//
// What the overview says needs attention, worst first, from everything the
// server reports about itself.

import 'package:flutter/widgets.dart';
import 'package:phosphor_icons/phosphor_icons.dart';

import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/admin_active_streams.data.gql.dart';
import '../../graphql/queries/__generated__/admin_indexing.data.gql.dart';
import '../../graphql/queries/__generated__/admin_transcoder.data.gql.dart';
import '../../graphql/queries/fragments/__generated__/system_status_fields.data.gql.dart';
import 'admin_controller.dart';
import 'admin_format.dart';

class AdminIssue {
  const AdminIssue({
    required this.bad,
    required this.icon,
    required this.title,
    required this.detail,
    required this.section,
  });

  final bool bad;
  final IconData icon;
  final String title;
  final String detail;
  final AdminSection section;
}

const _backends = {
  'software': 'Software',
  'vaapi': 'VAAPI',
  'nvenc': 'NVENC',
  'qsv': 'Quick Sync',
  'amf': 'AMF',
  'videotoolbox': 'VideoToolbox',
  'mf': 'Media Foundation',
  'cuda': 'CUDA (NVDEC)',
};

String backendLabel(String name) => _backends[name] ?? name;

String storagePlace(GSystemStatusFields_storage location) =>
    switch (location.kind) {
      GStorageLocationKind.DATA => 'Data folder',
      GStorageLocationKind.CACHE => 'Cache folder',
      GStorageLocationKind.TRANSCODE_CACHE => 'Transcode cache',
      GStorageLocationKind.IMAGE_CACHE => 'Image cache',
      GStorageLocationKind.EMBED_CACHE => 'Embedding cache',
      GStorageLocationKind.RECORDINGS => 'Recordings folder',
      _ => '${location.libraryName ?? 'Library'} folder',
    };

double memoryPercent(SystemStatus status) => ratio(
  status.memory.usedBytes,
  status.memory.cgroupLimitBytes ?? status.memory.totalBytes,
);

List<AdminIssue> adminIssues({
  required SystemStatus status,
  GAdminTranscoderData_transcoderInfo? transcoder,
  Iterable<GAdminIndexingData_scannerStatus> libraries = const [],
  Iterable<GAdminActiveStreamsData_activeStreams> streams = const [],
  DateTime? now,
}) {
  final out = <AdminIssue>[];
  void add(
    AdminSection section,
    IconData icon,
    String title, {
    bool bad = false,
    String detail = '',
  }) => out.add(
    AdminIssue(
      bad: bad,
      icon: icon,
      title: title,
      detail: detail,
      section: section,
    ),
  );

  final database = status.server.database;
  if (database.error != null || !database.healthy) {
    add(
      AdminSection.server,
      PhosphorIconsRegular.hardDrives,
      'Database unhealthy',
      bad: true,
      detail: database.error ?? database.notes.join('; '),
    );
  }
  final logs = status.logs;
  if (logs.errors > 0) {
    final last = logs.lastError;
    add(
      AdminSection.logs,
      PhosphorIconsRegular.warningCircle,
      '${plural(logs.errors, 'error')} logged since start',
      bad: true,
      detail: last == null
          ? ''
          : '${relative(last.at, now: now)}: ${last.message}',
    );
  }
  final plugins = status.plugins;
  if (plugins.circuitOpen > 0) {
    add(
      AdminSection.plugins,
      PhosphorIconsRegular.plug,
      '${plural(plugins.circuitOpen, 'plugin')} disabled after crashing',
      bad: true,
      detail: 'The circuit breaker opened; reload the plugin to recover.',
    );
  }
  if (plugins.loadFailures > 0) {
    add(
      AdminSection.plugins,
      PhosphorIconsRegular.plug,
      plural(plugins.loadFailures, 'plugin load failure'),
      detail: 'A .wasm in the search path couldn’t be loaded.',
    );
  }
  if (plugins.traps > 0) {
    add(
      AdminSection.plugins,
      PhosphorIconsRegular.plug,
      '${plural(plugins.traps, 'plugin trap')} since start',
      detail: 'A plugin crashed mid-call and was restarted.',
    );
  }
  final memory = memoryPercent(status);
  if (memory >= 90) {
    add(
      AdminSection.resources,
      PhosphorIconsRegular.stack,
      'Memory ${percent(memory)} used',
      bad: memory >= 96,
      detail: '${size(status.memory.availableBytes)} available',
    );
  }
  for (final location in status.storage) {
    final free = ratio(location.availableBytes, location.totalBytes);
    // Missing library roots surface through indexing; other folders are created
    // on first use.
    if (location.error != null && location.exists) {
      add(
        AdminSection.resources,
        PhosphorIconsRegular.hardDrives,
        '${storagePlace(location)} unavailable',
        bad: true,
        detail: '${location.path}: ${location.error}',
      );
    } else if ((location.totalBytes ?? 0) > 0 && free < 10) {
      add(
        AdminSection.resources,
        PhosphorIconsRegular.hardDrives,
        '${storagePlace(location)} nearly full',
        bad: free < 3,
        detail:
            '${size(location.availableBytes)} free of ${size(location.totalBytes)} on ${location.mountPoint ?? location.path}',
      );
    }
  }
  if (transcoder != null) {
    if (transcoder.ffmpeg.error != null) {
      add(
        AdminSection.transcoding,
        PhosphorIconsRegular.filmStrip,
        'ffmpeg not usable',
        bad: true,
        detail: transcoder.ffmpeg.error!,
      );
    }
    if (transcoder.ffprobe.error != null) {
      add(
        AdminSection.transcoding,
        PhosphorIconsRegular.filmStrip,
        'ffprobe not usable',
        bad: true,
        detail: transcoder.ffprobe.error!,
      );
    }
    if (transcoder.fellBack) {
      add(
        AdminSection.transcoding,
        PhosphorIconsRegular.filmStrip,
        'Configured ${transcoder.configuredHwaccel} acceleration unavailable',
        detail:
            'Transcoding on ${backendLabel(transcoder.resolvedBackend)} instead.',
      );
    }
    // A missing encoder costs its codec's rungs, not playback: H.264 always
    // remains.
    final missing = {
      for (final rung in transcoder.ladder)
        if (!rung.encoderAvailable) rung.encoder,
    };
    for (final encoder in missing) {
      add(
        AdminSection.transcoding,
        PhosphorIconsRegular.filmStrip,
        '$encoder encoder missing',
        detail: 'It isn’t in this ffmpeg build, so the ladder skips its rungs.',
      );
    }
  }
  for (final entry in libraries) {
    final name = entry.library.name;
    if (entry.watcher.state == GWatcherState.FAILED) {
      add(
        AdminSection.indexing,
        PhosphorIconsRegular.eye,
        '$name: watcher failed',
        bad: true,
        detail: entry.watcher.error ?? '',
      );
    }
    for (final root in entry.roots) {
      if (root.readable) continue;
      add(
        AdminSection.indexing,
        PhosphorIconsRegular.books,
        '$name: folder unreadable',
        bad: true,
        detail: root.error == null ? root.path : '${root.path} — ${root.error}',
      );
    }
    if (entry.scan.state == GScanState.FAILED) {
      add(
        AdminSection.indexing,
        PhosphorIconsRegular.arrowsClockwise,
        '$name: last scan failed',
        detail: entry.scan.error ?? '',
      );
    }
    if (entry.failureCount > 0) {
      final first = entry.recentFailures.isEmpty
          ? null
          : entry.recentFailures.first;
      add(
        AdminSection.indexing,
        PhosphorIconsRegular.warningCircle,
        '$name: ${plural(entry.failureCount, 'file')} failed to index',
        detail: first == null ? '' : '${first.stage} — ${first.path}',
      );
    }
  }
  for (final stream in streams) {
    if (stream.decision.route != GPlaybackRouteKind.TRANSCODE) continue;
    final job = stream.transcode;
    if (job == null) continue;
    final speed = job.speed;
    if (job.error != null) {
      add(
        AdminSection.playback,
        PhosphorIconsRegular.play,
        'Transcode failed for ${stream.user.name}',
        bad: true,
        detail: job.error!,
      );
    } else if (speed != null && speed < 1 && job.status == 'running') {
      add(
        AdminSection.playback,
        PhosphorIconsRegular.play,
        'Transcode below realtime (${speed.toStringAsFixed(2)}×)',
        detail:
            '${stream.user.name} · ${stream.work?.title ?? 'Live TV'} — playback will buffer.',
      );
    }
  }
  // Stable, so issues of one severity keep the order above.
  final bad = out.where((issue) => issue.bad);
  final rest = out.where((issue) => !issue.bad);
  return [...bad, ...rest];
}

/// One filesystem NoFlex depends on.
class FilesystemRow {
  FilesystemRow(
    this.key,
    this.fileSystem,
    this.total,
    this.available,
    this.used,
  );
  final String key;
  final String? fileSystem;
  final int total;
  final int? available;
  final int? used;
  final places = <String>[];
}

/// Data, cache and library folders usually share a few filesystems, and one row
/// per filesystem reads better than one per folder. Fullest first.
List<FilesystemRow> filesystems(SystemStatus status) {
  final groups = <String, FilesystemRow>{};
  for (final location in status.storage) {
    final total = location.totalBytes;
    if (total == null || total == 0) continue;
    final key = location.mountPoint ?? location.path;
    final group = groups.putIfAbsent(
      key,
      () => FilesystemRow(
        key,
        location.fileSystem,
        total,
        location.availableBytes,
        location.usedBytes,
      ),
    );
    final label = storagePlace(location);
    if (!group.places.contains(label)) group.places.add(label);
  }
  return groups.values.toList()
    ..sort((a, b) => ratio(b.used, b.total).compareTo(ratio(a.used, a.total)));
}
