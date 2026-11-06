// SPDX-License-Identifier: AGPL-3.0-only
//
// What the sidebar's status row says about the server, a port of the web's
// ServerStatus: polled while the row is on screen, and only for administrators,
// since nobody else may ask.

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/server_activity.data.gql.dart';
import '../../graphql/queries/__generated__/server_activity.req.gql.dart';
import '../../graphql/queries/__generated__/server_activity_scans.data.gql.dart';
import '../../graphql/queries/__generated__/server_activity_scans.req.gql.dart';
import '../../graphql/queries/__generated__/server_activity_versions.data.gql.dart';
import '../../graphql/queries/__generated__/server_activity_versions.req.gql.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../admin/admin_controller.dart';

enum ActivityKind { scan, transcode, playing, optimize, warn, bad }

class ActivityEntry {
  const ActivityEntry({
    required this.kind,
    required this.short,
    required this.title,
    required this.detail,
    this.section,
    this.progress = 0,
  });

  final ActivityKind kind;

  /// For the row itself, which has room for very little.
  final String short;
  final String title;
  final String detail;

  /// Where to look into it.
  final AdminSection? section;
  final double progress;
}

String _count(int n, String one, [String? many]) =>
    '$n ${n == 1 ? one : many ?? '${one}s'}';

/// One library being scanned, with its tally when it's that library alone.
typedef ScanRow = ({
  String library,
  ({int added, int updated, int removed})? run,
});
typedef StreamRow = ({String viewer, String? title, bool transcoding});
typedef VersionRow = ({bool running, double progress});

/// What's going on, innermost ring first.
List<ActivityEntry> activityRings({
  required int scanCount,
  required List<ScanRow> scanning,
  required List<StreamRow> streams,
  required List<VersionRow> versions,
  int queuedVersions = 0,
}) {
  String watchers(Iterable<StreamRow> rows) =>
      rows.map((row) => '${row.viewer} · ${row.title ?? 'Live TV'}').join(', ');
  // By the job, not the route: a stream outlives ffmpeg once the output is
  // finished.
  final transcoding = streams.where((row) => row.transcoding);
  final playing = streams.where((row) => !row.transcoding);
  final optimizing = versions.where((row) => row.running);
  final queued = queuedVersions;
  final run = scanning.length == 1 ? scanning.first.run : null;
  return [
    if (scanCount > 0)
      ActivityEntry(
        kind: ActivityKind.scan,
        short: 'Scanning',
        title: scanning.length == 1
            ? 'Scanning ${scanning.first.library}'
            : 'Scanning ${_count(scanCount, 'library', 'libraries')}',
        detail: run == null
            ? 'Looking for changes'
            : '${run.added} added · ${run.updated} updated · ${run.removed} removed',
        section: AdminSection.indexing,
      ),
    if (transcoding.isNotEmpty)
      ActivityEntry(
        kind: ActivityKind.transcode,
        short: '${transcoding.length} transcoding',
        title: '${_count(transcoding.length, 'stream')} transcoding',
        detail: watchers(transcoding),
        section: AdminSection.playback,
      ),
    if (playing.isNotEmpty)
      ActivityEntry(
        kind: ActivityKind.playing,
        short: '${playing.length} playing',
        title: '${_count(playing.length, 'stream')} playing',
        detail: watchers(playing),
        section: AdminSection.playback,
      ),
    if (optimizing.isNotEmpty)
      ActivityEntry(
        kind: ActivityKind.optimize,
        short: 'Optimizing',
        title: 'Optimizing ${_count(optimizing.length, 'version')}',
        detail: queued > 0 ? '$queued more queued' : 'Nothing else queued',
        progress:
            optimizing.fold(0.0, (total, row) => total + row.progress) /
            optimizing.length,
        section: AdminSection.transcoding,
      ),
  ];
}

/// What's wrong, worst first.
List<ActivityEntry> activityProblems({
  required bool unreachable,
  required bool? databaseHealthy,
  required int pluginsPaused,
}) => [
  if (unreachable)
    const ActivityEntry(
      kind: ActivityKind.bad,
      short: 'Can’t reach the server',
      title: 'Can’t reach the server',
      detail: 'Status will return when it answers.',
    ),
  if (databaseHealthy == false)
    const ActivityEntry(
      kind: ActivityKind.bad,
      short: 'Database unhealthy',
      title: 'The database is unhealthy',
      detail: 'See the server section for details.',
      section: AdminSection.server,
    ),
  if (pluginsPaused > 0)
    ActivityEntry(
      kind: ActivityKind.warn,
      short: 'Plugin trouble',
      title: '${_count(pluginsPaused, 'plugin')} paused after failures',
      detail: 'See the plugins section for details.',
      section: AdminSection.plugins,
    ),
];

/// The row's second line: trouble outranks activity, which outranks the version.
String activitySummary({
  required List<ActivityEntry> problems,
  required List<ActivityEntry> rings,
  required String idle,
}) {
  if (problems.isNotEmpty) return problems.first.short;
  if (rings.isNotEmpty) return rings.map((ring) => ring.short).join(' · ');
  return idle;
}

class ServerActivity {
  ServerActivity() {
    unawaited(_pollActivity());
    unawaited(_pollVersions());
    _activityTimer = Timer.periodic(_every, (_) => unawaited(_pollActivity()));
    // The whole table, every row, so it runs at a gentler pace.
    _versionsTimer = Timer.periodic(
      _every * 3,
      (_) => unawaited(_pollVersions()),
    );
  }

  static const _every = Duration(seconds: 5);

  final _graphql = getIt<GraphqlService>();
  late final Timer _activityTimer;
  late final Timer _versionsTimer;
  var _closed = false;

  final _activity = signal<GServerActivityData?>(null);
  final _scans = signal<GServerActivityScansData?>(null);
  final _versions = signal<GServerActivityVersionsData?>(null);
  final _unreachable = signal(false);

  late final problems = computed(() {
    final status = _activity.value?.systemStatus;
    return activityProblems(
      unreachable: _unreachable.value,
      databaseHealthy: _unreachable.value
          ? null
          : status?.server.database.healthy,
      pluginsPaused: _unreachable.value ? 0 : status?.plugins.circuitOpen ?? 0,
    );
  });

  late final rings = computed(() {
    if (_unreachable.value) return const <ActivityEntry>[];
    final data = _activity.value;
    final scanCount = data?.systemStatus.activeScans ?? 0;
    final scans = _scans.value;
    final versions = _versions.value;
    return activityRings(
      scanCount: scanCount,
      scanning: [
        if (scanCount > 0 && scans != null)
          for (final row in scans.scannerStatus)
            if (row.activeScan != null || row.scan.state == GScanState.RUNNING)
              (
                library: row.library.name,
                run: row.activeScan == null
                    ? null
                    : (
                        added: row.activeScan!.added,
                        updated: row.activeScan!.updated,
                        removed: row.activeScan!.removed,
                      ),
              ),
      ],
      streams: [
        if (data != null)
          for (final stream in data.activeStreams)
            (
              viewer: stream.user.name,
              title: stream.work?.title,
              transcoding: stream.transcode?.status == 'running',
            ),
      ],
      versions: [
        if (versions != null)
          // Running ones lead the list, so the first few hold them all.
          for (final version in versions.optimizedVersions.nodes)
            (
              running: version.status == GOptimizedVersionStatus.RUNNING,
              progress: version.progress,
            ),
      ],
    );
  });

  late final summary = computed(() {
    final session = getIt<SessionService>();
    return activitySummary(
      problems: problems.value,
      rings: rings.value,
      idle: 'v${session.serverVersion.value} · ${session.serverBackend.value}',
    );
  });

  Future<void> _pollActivity() async {
    try {
      final data = await _graphql.run(
        GServerActivityReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
      );
      if (_closed) return;
      batch(() {
        _activity.value = data;
        _unreachable.value = false;
      });
      // Stats every library root, so only ask while there's a scan to name.
      if (data.systemStatus.activeScans > 0) await _pollScans();
    } on Exception {
      if (!_closed) _unreachable.value = true;
    }
  }

  Future<void> _pollScans() async {
    final data = await _graphql.run(
      GServerActivityScansReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    if (!_closed) _scans.value = data;
  }

  Future<void> _pollVersions() async {
    try {
      final data = await _graphql.run(
        GServerActivityVersionsReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
      );
      if (!_closed) _versions.value = data;
    } on Exception {
      // The activity poll says when the server's away; this keeps its last answer.
    }
  }

  void dispose() {
    _closed = true;
    _activityTimer.cancel();
    _versionsTimer.cancel();
  }
}
