// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/admin/admin_controller.dart';
import 'package:noflex/features/shell/server_activity.dart';

void main() {
  test('streams divide by the job, and live TV has no title to name', () {
    final rings = activityRings(
      scanCount: 0,
      scanning: const [],
      streams: const [
        (viewer: 'ada', title: 'Heat', transcoding: true),
        (viewer: 'bo', title: null, transcoding: false),
      ],
      versions: const [],
    );
    expect(rings.map((ring) => ring.kind), [
      ActivityKind.transcode,
      ActivityKind.playing,
    ]);
    expect(rings.first.detail, 'ada · Heat');
    expect(rings.last.detail, 'bo · Live TV');
    expect(rings.first.section, AdminSection.playback);
  });

  test('optimizing averages what runs and counts what waits', () {
    final ring = activityRings(
      scanCount: 0,
      scanning: const [],
      streams: const [],
      versions: const [
        (running: true, queued: false, progress: 0.2),
        (running: true, queued: false, progress: 0.6),
        (running: false, queued: true, progress: 0),
        (running: false, queued: false, progress: 1),
      ],
    ).single;
    expect(ring.progress, closeTo(0.4, 1e-9));
    expect(ring.detail, '1 more queued');
  });

  test('a scan names its library only when it’s one, tally included', () {
    ActivityEntry scan(List<ScanRow> rows, int count) => activityRings(
      scanCount: count,
      scanning: rows,
      streams: const [],
      versions: const [],
    ).single;

    final one = scan(const [
      (library: 'Films', run: (added: 3, updated: 1, removed: 0)),
    ], 1);
    expect(one.title, 'Scanning Films');
    expect(one.detail, '3 added · 1 updated · 0 removed');

    // The count can arrive before the rows that name them.
    final unnamed = scan(const [], 2);
    expect(unnamed.title, 'Scanning 2 libraries');
    expect(unnamed.detail, 'Looking for changes');
  });

  test('the row says trouble over activity, and the version when idle', () {
    final busy = activityRings(
      scanCount: 1,
      scanning: const [],
      streams: const [(viewer: 'ada', title: 'Heat', transcoding: true)],
      versions: const [],
    );
    final trouble = activityProblems(
      unreachable: false,
      databaseHealthy: false,
      pluginsPaused: 2,
    );
    expect(
      activitySummary(problems: trouble, rings: busy, idle: 'v1'),
      'Database unhealthy',
    );
    expect(
      activitySummary(problems: const [], rings: busy, idle: 'v1'),
      'Scanning · 1 transcoding',
    );
    expect(
      activitySummary(problems: const [], rings: const [], idle: 'v1'),
      'v1',
    );
  });
}
