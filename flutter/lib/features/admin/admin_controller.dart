// SPDX-License-Identifier: AGPL-3.0-only
//
// The administration area's shared state: which section is open, the server's
// libraries, users and plugins, and the live host sample every diagnostic
// section draws from. Sections own whatever else they ask for, so it stops when
// they're closed.

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/widgets.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/queries/__generated__/admin.req.gql.dart';
import '../../graphql/queries/__generated__/admin_status_updates.data.gql.dart';
import '../../graphql/queries/__generated__/admin_status_updates.req.gql.dart';
import '../../graphql/queries/__generated__/admin_status_updates.var.gql.dart';
import '../../graphql/queries/__generated__/admin_system_status.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/system_status_fields.data.gql.dart';
import '../../graphql/query.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../home/home_service.dart';

typedef SystemStatus = GSystemStatusFields;

enum AdminSection {
  overview('Overview', PhosphorIconsRegular.squaresFour),
  activity('Activity', PhosphorIconsRegular.trendUp),
  playback('Playback', PhosphorIconsRegular.play),
  libraries('Libraries', PhosphorIconsRegular.books),
  indexing('Indexing', PhosphorIconsRegular.magnifyingGlass),
  transcoding('Transcoding', PhosphorIconsRegular.filmStrip),
  resources('Resources', PhosphorIconsRegular.gauge),
  plugins('Plugins', PhosphorIconsRegular.plug),
  users('Users', PhosphorIconsRegular.users),
  logs('Logs', PhosphorIconsRegular.listDashes),
  server('Server', PhosphorIconsRegular.hardDrives);

  const AdminSection(this.label, this.icon);
  final String label;
  final IconData icon;
}

enum LiveMode {
  connecting('Connecting…', 'Waiting for the first sample.'),
  live('Live', 'Streaming over WebSocket, one sample every 2 s.'),
  polling(
    'Polling',
    'The live connection is unavailable; refreshing every 5 s.',
  ),
  offline('Offline', 'The server isn’t answering.');

  const LiveMode(this.label, this.detail);
  final String label;
  final String detail;
}

/// One reading for the sparklines.
class StatusPoint {
  const StatusPoint({
    required this.at,
    required this.cpuPercent,
    required this.memoryUsedBytes,
    required this.processCpuPercent,
    required this.rxBytesPerSec,
    required this.txBytesPerSec,
  });

  final DateTime at;
  final double cpuPercent;
  final int memoryUsedBytes;
  final double processCpuPercent;
  final double rxBytesPerSec;
  final double txBytesPerSec;
}

/// [history] with [point] added: in time order, and never longer than the
/// server's own window. A sample older than the newest is a duplicate delivery.
List<StatusPoint> withSample(
  List<StatusPoint> history,
  StatusPoint point, {
  int limit = 180,
}) {
  if (history.isNotEmpty && !history.last.at.isBefore(point.at)) return history;
  final kept = history.length >= limit
      ? history.sublist(history.length - limit + 1)
      : history;
  return [...kept, point];
}

/// The host sample, pushed by a subscription and, when that goes quiet behind a
/// proxy without WebSocket upgrades or a dropped socket, polled instead.
class LiveStatus {
  LiveStatus() {
    final client = getIt<GraphqlService>().client;
    _updates = Query(
      client,
      GAdminStatusUpdatesReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    _stopUpdates = effect(() {
      final next = _updates.data.value?.systemStatusUpdates;
      if (next != null) untracked(() => _apply(next, LiveMode.live));
    });
    unawaited(_refresh());
    _poll = Timer.periodic(_pollEvery, (_) {
      if (DateTime.now().difference(_updatedAt) <= _staleAfter) return;
      if (mode.value == LiveMode.live) mode.value = LiveMode.polling;
      unawaited(_refresh());
    });
  }

  static const _pollEvery = Duration(seconds: 5);

  /// A pushed sample older than this means the socket has stopped delivering.
  static const _staleAfter = Duration(milliseconds: 6500);

  final status = signal<SystemStatus?>(null);
  final history = signal<List<StatusPoint>>(const []);
  final mode = signal(LiveMode.connecting);
  final error = signal<String?>(null);

  late final Query<GAdminStatusUpdatesData, GAdminStatusUpdatesVars> _updates;
  late final void Function() _stopUpdates;
  late final Timer _poll;
  var _updatedAt = DateTime.fromMillisecondsSinceEpoch(0);
  var _closed = false;

  void _apply(SystemStatus next, LiveMode via) {
    batch(() {
      status.value = next;
      mode.value = via;
      error.value = null;
      history.value = withSample(
        history.value,
        StatusPoint(
          at: next.sampledAt,
          cpuPercent: next.cpu.usagePercent,
          memoryUsedBytes: next.memory.usedBytes,
          processCpuPercent: next.process.cpuPercent,
          rxBytesPerSec: next.networkTotal.rxBytesPerSec,
          txBytesPerSec: next.networkTotal.txBytesPerSec,
        ),
      );
    });
    _updatedAt = DateTime.now();
  }

  Future<void> _refresh() async {
    try {
      final data = await getIt<GraphqlService>().run(
        GAdminSystemStatusReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
      );
      if (_closed) return;
      // The server's history is authoritative after a gap.
      history.value = [
        for (final sample in data.systemStatusHistory)
          StatusPoint(
            at: sample.at,
            cpuPercent: sample.cpuPercent,
            memoryUsedBytes: sample.memoryUsedBytes,
            processCpuPercent: sample.processCpuPercent,
            rxBytesPerSec: sample.rxBytesPerSec,
            txBytesPerSec: sample.txBytesPerSec,
          ),
      ];
      _apply(
        data.systemStatus,
        mode.value == LiveMode.live ? LiveMode.live : LiveMode.polling,
      );
    } on Exception catch (cause) {
      if (_closed) return;
      batch(() {
        error.value = cause.toString();
        if (status.value == null ||
            DateTime.now().difference(_updatedAt) > _staleAfter * 2) {
          mode.value = LiveMode.offline;
        }
      });
    }
  }

  void dispose() {
    _closed = true;
    _poll.cancel();
    _stopUpdates();
    _updates.dispose();
  }
}

class AdminController {
  /// Null on a phone's section list; side by side it reads as the overview.
  final section = signal<AdminSection?>(null);

  final admin = Query(
    getIt<GraphqlService>().client,
    GAdminReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
  );

  final live = LiveStatus();

  void open(AdminSection next) => section.value = next;

  /// A scan or a library change alters what every list holds.
  void libraryContentChanged() {
    unawaited(getIt<SessionService>().refreshLibraries());
    getIt<HomeService>().refresh();
  }

  void dispose() {
    admin.dispose();
    live.dispose();
  }
}
