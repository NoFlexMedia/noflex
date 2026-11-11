// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/admin/admin_controller.dart';
import 'package:noflex/features/admin/admin_format.dart';
import 'package:noflex/features/admin/admin_issues.dart';
import 'package:noflex/features/admin/admin_plugins.dart';
import 'package:noflex/features/admin/admin_widgets.dart';
import 'package:noflex/graphql/queries/fragments/__generated__/system_status_fields.data.gql.dart';

const _gib = 1024 * 1024 * 1024;

Map<String, dynamic> _location(
  String path, {
  String kind = 'DATA',
  int? total = 100 * _gib,
  int? available = 50 * _gib,
  String? mount = '/',
  String? error,
  bool exists = true,
}) => {
  '__typename': 'StorageLocation',
  'kind': kind,
  'path': path,
  'libraryId': null,
  'libraryName': null,
  'exists': exists,
  'mountPoint': mount,
  'fileSystem': 'btrfs',
  'totalBytes': total,
  'availableBytes': available,
  'usedBytes': total == null || available == null ? null : total - available,
  'contentBytes': null,
  'error': error,
};

GSystemStatusFieldsData _status({
  int errors = 0,
  int circuitOpen = 0,
  int loadFailures = 0,
  int memoryUsed = 4 * _gib,
  bool healthy = true,
  List<Map<String, dynamic>> storage = const [],
}) => GSystemStatusFieldsData.fromJson({
  '__typename': 'SystemStatus',
  'sampledAt': '2026-11-05T12:00:00Z',
  'activeScans': 0,
  'server': {
    '__typename': 'ServerRuntime',
    'version': '0.1.0',
    'startedAt': '2026-11-05T11:00:00Z',
    'uptimeSeconds': 3600,
    'bind': '0.0.0.0:28793',
    'publicUrl': null,
    'dataDir': '/data',
    'cacheDir': '/cache',
    'config': <Object>[],
    'database': {
      '__typename': 'DatabaseStatus',
      'backend': 'turso',
      'engineVersion': '1',
      'schemaVersion': 1,
      'healthy': healthy,
      'notes': ['wal checkpoint stuck'],
      'path': null,
      'sizeBytes': null,
      'walSizeBytes': null,
      'error': null,
    },
  },
  'host': {
    '__typename': 'HostInfo',
    'hostname': 'box',
    'osName': 'Linux',
    'osVersion': null,
    'kernelVersion': null,
    'arch': 'x86_64',
    'bootedAt': null,
    'uptimeSeconds': null,
  },
  'cpu': {
    '__typename': 'CpuStatus',
    'usagePercent': 5.0,
    'brand': 'CPU',
    'physicalCores': 4,
    'logicalCores': 8,
    'loadAverage': null,
    'cores': <Object>[],
  },
  'memory': {
    '__typename': 'MemoryStatus',
    'totalBytes': 8 * _gib,
    'usedBytes': memoryUsed,
    'availableBytes': 8 * _gib - memoryUsed,
    'swapTotalBytes': 0,
    'swapUsedBytes': 0,
    'cgroupLimitBytes': null,
  },
  'process': {
    '__typename': 'ProcessStatus',
    'pid': 1,
    'cpuPercent': 1.0,
    'cpuPercentOfHost': 0.1,
    'rssBytes': _gib,
    'virtualBytes': _gib,
    'threads': null,
    'openFiles': null,
    'openFilesLimit': null,
  },
  'network': <Object>[],
  'networkTotal': {
    '__typename': 'NetworkTotals',
    'rxBytesPerSec': 0.0,
    'txBytesPerSec': 0.0,
  },
  'storage': storage,
  'plugins': {
    '__typename': 'PluginHealthSummary',
    'loaded': 2,
    'circuitOpen': circuitOpen,
    'instanceRestarts': 0,
    'loadFailures': loadFailures,
    'traps': 0,
  },
  'logs': {
    '__typename': 'LogSummary',
    'warnings': 0,
    'errors': errors,
    'lastError': null,
  },
})!;

StatusPoint _point(int second) => StatusPoint(
  at: DateTime.utc(2026, 1, 1, 0, 0, second),
  cpuPercent: 0,
  memoryUsedBytes: 0,
  processCpuPercent: 0,
  rxBytesPerSec: 0,
  txBytesPerSec: 0,
);

void main() {
  group('readings', () {
    test('zero is a reading and null is not', () {
      expect(size(0), '0 B');
      expect(size(null), '—');
      expect(rate(0), '0 b/s');
      expect(rate(null), '—');
      expect(percent(0), '0%');
    });

    test('sizes are binary, with decimals from gigabytes up', () {
      expect(size(1536), '2 KB');
      expect(size(5 * 1024 * 1024), '5 MB');
      expect(size(_gib + _gib ~/ 2), '1.5 GB');
    });

    test('rates are the bits a link is rated in', () {
      expect(rate(125), '1.0 Kb/s');
      expect(rate(1250000), '10 Mb/s');
      expect(rate(125000000), '1.0 Gb/s');
    });

    test('a ratio of nothing is zero, and never past full', () {
      expect(ratio(5, 0), 0);
      expect(ratio(null, 10), 0);
      expect(ratio(20, 10), 100);
    });

    test('uptime shows its two largest units', () {
      expect(uptime(93784), '1d 2h');
      expect(uptime(3720), '1h 2m');
      expect(uptime(90), '1m 30s');
      expect(uptime(42), '42s');
    });

    test('watched time reads in the unit that suits it', () {
      expect(watched(0), '0 min');
      expect(watched(30000), '<1 min');
      expect(watched(12 * 60000), '12 min');
      expect(watched(204 * 60000), '3.4 h');
    });

    test('a meter changes color at its thresholds', () {
      expect(meterTone(79), MeterTone.ok);
      expect(meterTone(80), MeterTone.warn);
      expect(meterTone(92), MeterTone.bad);
      expect(meterTone(85, warn: 85, bad: 95), MeterTone.warn);
    });

    test('a pixel budget a little under a tier still reads as that tier', () {
      expect(pixelTier(1920 * 1080), '~1080p');
      expect(pixelTier(3840 * 2160), '~4K');
      expect(pixelTier((3840 * 2160 * 0.95).round()), '~4K');
    });

    test('only episodes get a season and number', () {
      expect(
        playedTitle(
          kind: 'EPISODE',
          title: 'Pilot',
          parentIndex: 1,
          indexNumber: 3,
        ),
        'S1 · E3 · Pilot',
      );
      expect(playedTitle(kind: 'MOVIE', title: 'Film', indexNumber: 3), 'Film');
    });

    test('what hasn’t happened yet reads forward', () {
      final now = DateTime.utc(2026);
      expect(relative(now.add(const Duration(hours: 2)), now: now), 'in 2h');
      expect(
        relative(now.subtract(const Duration(minutes: 3)), now: now),
        '3m ago',
      );
      expect(relative(null, now: now), 'never');
    });
  });

  group('chartTicks', () {
    test('an empty chart still has an axis', () {
      expect(chartTicks(0, whole: true), [0, 1, 2, 3, 4]);
      expect(chartTicks(0, whole: false), [0, 0.25, 0.5, 0.75, 1]);
    });

    test('steps are clean and hold the peak', () {
      expect(chartTicks(37, whole: true), [0, 10, 20, 30, 40]);
      expect(chartTicks(90, whole: true), [0, 25, 50, 75, 100]);
    });

    test('plays never step by less than one', () {
      expect(chartTicks(1, whole: true), [0, 1, 2, 3, 4]);
    });

    test('hours can step by fractions', () {
      expect(chartTicks(1, whole: false), [0, 0.25, 0.5, 0.75, 1]);
    });
  });

  group('foldRepeats', () {
    bool same(String a, String b) => a[0] == b[0];

    test('folds neighbors and keeps the oldest', () {
      final rows = foldRepeats(['a1', 'a2', 'a3', 'b1'], same);
      expect(rows, hasLength(2));
      expect(rows.first.first, 'a1');
      expect(rows.first.oldest, 'a3');
      expect(rows.first.count, 3);
      expect(rows.last.count, 1);
    });

    test('the same line apart from itself stays apart', () {
      expect(foldRepeats(['a1', 'b1', 'a2'], same), hasLength(3));
    });
  });

  group('nextScan', () {
    final now = DateTime.utc(2026, 1, 2);

    test('a manual library has no next scan', () {
      expect(
        nextScan(schedule: 'DISABLED', last: now, now: now).label,
        'Only when you scan',
      );
    });

    test('a library never scanned is due', () {
      expect(
        nextScan(schedule: 'DAILY', last: null, now: now).label,
        'Due now',
      );
    });

    test('an overdue library is due', () {
      expect(
        nextScan(
          schedule: 'HOURLY',
          last: now.subtract(const Duration(hours: 2)),
          now: now,
        ).label,
        'Due now',
      );
    });

    test('otherwise it counts down', () {
      expect(
        nextScan(
          schedule: 'DAILY',
          last: now.subtract(const Duration(hours: 20)),
          now: now,
        ).label,
        'in 4h',
      );
    });
  });

  group('groupSessions', () {
    String userOf((String, int) session) => session.$1;

    test('people in order of their newest, each newest first', () {
      final groups = groupSessions(
        [('b', 1), ('a', 2), ('b', 3), ('a', 4)],
        userOf: userOf,
        page: 0,
        pageSize: 10,
      );
      expect(groups.map((g) => g.userId), ['b', 'a']);
      expect(groups.first.sessions.map((s) => s.$2), [1, 3]);
      expect(groups.last.sessions.map((s) => s.$2), [2, 4]);
    });

    test('a page keeps a person’s whole count', () {
      final sessions = [('a', 1), ('a', 2), ('a', 3), ('b', 4)];
      final second = groupSessions(
        sessions,
        userOf: userOf,
        page: 1,
        pageSize: 2,
      );
      expect(second.map((g) => g.userId), ['a', 'b']);
      expect(second.first.total, 3);
      expect(second.first.sessions, hasLength(1));
    });
  });

  group('withSample', () {
    test('appends what’s newer', () {
      final next = withSample([_point(1)], _point(2));
      expect(next, hasLength(2));
    });

    test('ignores a redelivered or older sample', () {
      final history = [_point(1), _point(2)];
      expect(withSample(history, _point(2)), same(history));
      expect(withSample(history, _point(1)), same(history));
    });

    test('never grows past the server’s window', () {
      final next = withSample(
        [_point(1), _point(2), _point(3)],
        _point(4),
        limit: 3,
      );
      expect(next.map((p) => p.at.second), [2, 3, 4]);
    });
  });

  group('adminIssues', () {
    test('nothing wrong, nothing said', () {
      expect(adminIssues(status: _status()), isEmpty);
    });

    test('the worst comes first', () {
      final issues = adminIssues(
        status: _status(errors: 2, loadFailures: 1, healthy: false),
      );
      expect(issues.map((i) => i.bad), [true, true, false]);
      expect(issues.first.section, AdminSection.server);
      expect(issues.first.detail, 'wal checkpoint stuck');
      expect(issues[1].title, '2 errors logged since start');
      expect(issues.last.title, '1 plugin load failure');
    });

    test('memory warns at 90% and is bad at 96%', () {
      expect(
        adminIssues(status: _status(memoryUsed: (8 * _gib * 0.89).round())),
        isEmpty,
      );
      expect(
        adminIssues(status: _status(memoryUsed: (8 * _gib * 0.91).round()))
            .single
            .bad,
        isFalse,
      );
      expect(
        adminIssues(status: _status(memoryUsed: (8 * _gib * 0.97).round()))
            .single
            .bad,
        isTrue,
      );
    });

    test('a nearly full disk warns, then is bad', () {
      final warned = adminIssues(
        status: _status(storage: [_location('/data', available: 9 * _gib)]),
      ).single;
      expect(warned.bad, isFalse);
      expect(warned.title, 'Data folder nearly full');
      final bad = adminIssues(
        status: _status(storage: [_location('/data', available: 2 * _gib)]),
      ).single;
      expect(bad.bad, isTrue);
    });

    test('a folder not yet created isn’t a problem until it exists', () {
      expect(
        adminIssues(
          status: _status(
            storage: [
              _location(
                '/recordings',
                kind: 'RECORDINGS',
                total: null,
                available: null,
                mount: null,
                exists: false,
                error: 'No such file',
              ),
            ],
          ),
        ),
        isEmpty,
      );
      expect(
        adminIssues(
          status: _status(
            storage: [_location('/data', error: 'Permission denied')],
          ),
        ).single.title,
        'Data folder unavailable',
      );
    });
  });

  group('filesystems', () {
    test('folders on one mount are one row, fullest first', () {
      final rows = filesystems(
        _status(
          storage: [
            _location('/data', mount: '/a', available: 90 * _gib),
            _location(
              '/cache',
              kind: 'CACHE',
              mount: '/a',
              available: 90 * _gib,
            ),
            _location(
              '/media',
              kind: 'LIBRARY_ROOT',
              mount: '/b',
              available: 10 * _gib,
            ),
          ],
        ),
      );
      expect(rows.map((r) => r.key), ['/b', '/a']);
      expect(rows.last.places, ['Data folder', 'Cache folder']);
    });

    test('what has no size has no row', () {
      expect(
        filesystems(_status(storage: [_location('/x', total: null)])),
        isEmpty,
      );
    });
  });

  group('pluginHealth', () {
    test('a crash outranks a failing task, which outranks a past restart', () {
      expect(
        pluginHealth(circuitOpen: true, taskFailing: true, restarts: 3).tone,
        StateTone.bad,
      );
      expect(
        pluginHealth(circuitOpen: false, taskFailing: true, restarts: 3).label,
        'Task failing',
      );
      expect(
        pluginHealth(circuitOpen: false, taskFailing: false, restarts: 1).label,
        'Recovered',
      );
      expect(
        pluginHealth(circuitOpen: false, taskFailing: false, restarts: 0).label,
        'Healthy',
      );
    });
  });
}
