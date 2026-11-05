// SPDX-License-Identifier: AGPL-3.0-only
//
// Live TV's state: channels and the DVR from one query, the guide fetched a
// window at a time, and the clock both follow. There's no subscription for any
// of it, so it polls as the web does.

import 'dart:async';

import 'package:built_collection/built_collection.dart';
import 'package:ferry/ferry.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/cancel_recording_schedule.req.gql.dart';
import '../../graphql/queries/__generated__/delete_recording.req.gql.dart';
import '../../graphql/queries/__generated__/live_tv.data.gql.dart';
import '../../graphql/queries/__generated__/live_tv.req.gql.dart';
import '../../graphql/queries/__generated__/live_tv.var.gql.dart';
import '../../graphql/queries/__generated__/live_tv_guide.req.gql.dart';
import '../../graphql/queries/__generated__/schedule_recording.req.gql.dart';
import '../../graphql/query.dart';
import '../../services/graphql.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
import 'guide_math.dart';

typedef LiveChannel = GLiveTvData_liveTvChannels_nodes;
typedef LiveSchedule = GLiveTvData_recordingSchedules_nodes;
typedef LiveRecording = GLiveTvData_recordings_nodes;

/// M3U files leave these blank as often as they leave them out.
extension LiveChannelLabels on LiveChannel {
  String? get numberLabel => (number?.isEmpty ?? true) ? null : number;
  String? get groupLabel => (group?.isEmpty ?? true) ? null : group;
}

const _livePoll = Duration(seconds: 30);
const _guideRefresh = Duration(minutes: 5);

/// The server refuses a guide request for more channels than this.
const _maxGuideChannels = 200;

class LiveTvController {
  LiveTvController({required this.shouldPoll}) {
    _stopFollowingGuide = effect(() {
      final hasChannels = _channelKey.value.isNotEmpty;
      anchor.value;
      if (hasChannels) untracked(() => unawaited(loadGuide()));
    });
    _poll = Timer.periodic(_livePoll, (_) => _tick());
    _ticker = Timer.periodic(
      const Duration(seconds: 1),
      (_) => clock.value = DateTime.now(),
    );
  }

  /// Whether anyone can see this page right now.
  final bool Function() shouldPoll;

  GraphqlService get _graphql => getIt<GraphqlService>();
  ToastService get _toasts => getIt<ToastService>();

  late final live = Query<GLiveTvData, GLiveTvVars>(
    _graphql.client,
    GLiveTvReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
  );

  /// Moves with the header's clock.
  final clock = signal(DateTime.now());

  /// What the guide draws against; moves less often than [clock] so a row
  /// isn't rebuilt every second.
  final now = signal(DateTime.now());
  final anchor = signal(guideAnchor(DateTime.now()));
  final group = signal<String?>(null);

  final programs = signal<List<GuideProgram>>(const []);
  final guideLoading = signal(false);
  final guideError = signal<Exception?>(null);

  late final Timer _poll;
  late final Timer _ticker;
  late final void Function() _stopFollowingGuide;
  int _guideGeneration = 0;
  DateTime _guideAt = DateTime.now();

  late final windowEnd = computed(() => anchor.value.add(guideSpan));

  late final channels = computed<List<LiveChannel>>(() {
    final nodes = live.data.value?.liveTvChannels.nodes;
    if (nodes == null) return const [];
    return nodes.where((channel) => channel.enabled).toList()
      ..sort((a, b) => compareChannels(a.number, a.name, b.number, b.name));
  });

  late final groups = computed<List<String>>(() {
    final names = {for (final channel in channels.value) ?channel.groupLabel};
    return names.toList()..sort();
  });

  late final visibleChannels = computed<List<LiveChannel>>(() {
    final chosen = group.value;
    return chosen == null
        ? channels.value
        : channels.value.where((c) => c.groupLabel == chosen).toList();
  });

  late final channelById = computed<Map<String, LiveChannel>>(
    () => {for (final channel in channels.value) channel.id: channel},
  );

  /// Only changes when the channels do, unlike the list behind it.
  late final _channelKey = computed(
    () => channels.value
        .take(_maxGuideChannels)
        .map((channel) => channel.id)
        .join(','),
  );

  late final rows = computed(
    () => guideRows(programs.value, start: anchor.value, end: windowEnd.value),
  );

  late final programsByChannel = computed<Map<String, List<GuideProgram>>>(() {
    final byChannel = <String, List<GuideProgram>>{};
    for (final program in programs.value) {
      (byChannel[program.channelId] ??= []).add(program);
    }
    return byChannel;
  });

  late final programById = computed<Map<String, GuideProgram>>(
    () => {for (final program in programs.value) program.id: program},
  );

  late final schedules = computed<List<LiveSchedule>>(() {
    final nodes = live.data.value?.recordingSchedules.nodes;
    if (nodes == null) return const [];
    return nodes.toList()..sort((a, b) {
      final upcoming = (_upcoming(b) ? 1 : 0).compareTo(_upcoming(a) ? 1 : 0);
      return upcoming != 0 ? upcoming : a.startAt.compareTo(b.startAt);
    });
  });

  late final recordings = computed<List<LiveRecording>>(() {
    final nodes = live.data.value?.recordings.nodes;
    if (nodes == null) return const [];
    return nodes.toList()..sort(
      (a, b) =>
          (b.startedAt ?? b.createdAt).compareTo(a.startedAt ?? a.createdAt),
    );
  });

  /// Programs set to record.
  late final scheduled = computed<Set<String>>(
    () => {
      for (final schedule in schedules.value)
        if (_upcoming(schedule) && schedule.programId != null)
          schedule.programId!,
    },
  );

  /// Nothing to show on any tab, so the page says so rather than offering them.
  late final isEmpty = computed(
    () =>
        channels.value.isEmpty &&
        schedules.value.isEmpty &&
        recordings.value.isEmpty,
  );

  static bool _upcoming(LiveSchedule schedule) =>
      schedule.status == GRecordingScheduleStatus.SCHEDULED;

  void _tick() {
    if (!shouldPoll()) return;
    final at = DateTime.now();
    batch(() {
      now.value = at;
      if (guideWindowIsStale(anchor.value, at)) anchor.value = guideAnchor(at);
    });
    live.refetch();
    if (at.difference(_guideAt) > _guideRefresh) unawaited(loadGuide());
  }

  /// Every page of the window, so no channel's row comes up short.
  Future<void> loadGuide() async {
    final ids = _channelKey.value;
    if (ids.isEmpty) return;
    final generation = ++_guideGeneration;
    final start = anchor.value;
    final end = windowEnd.value;
    _guideAt = DateTime.now();
    batch(() {
      guideLoading.value = true;
      guideError.value = null;
    });
    try {
      final loaded = <GuideProgram>[];
      String? cursor;
      do {
        final after = cursor;
        final page = await _graphql.run(
          GLiveTvGuideReq(
            (b) => b
              ..fetchPolicy = FetchPolicy.NetworkOnly
              ..vars.channelIds = ListBuilder(ids.split(','))
              ..vars.start = start
              ..vars.end = end
              ..vars.after = after,
          ),
        );
        if (generation != _guideGeneration) return;
        loaded.addAll([
          for (final node in page.liveTvGuide.nodes)
            GuideProgram(
              id: node.id,
              channelId: node.channelId,
              title: node.title,
              subtitle: node.subtitle,
              description: node.description,
              category: node.category,
              start: node.startAt,
              end: node.endAt,
            ),
        ]);
        final info = page.liveTvGuide.pageInfo;
        cursor = info.hasNextPage ? info.endCursor : null;
      } while (cursor != null);
      programs.value = loaded;
    } on SessionExpired {
      // The expiry handler has moved to the sign-in screen.
    } on Exception catch (cause) {
      if (generation == _guideGeneration) guideError.value = cause;
    } finally {
      if (generation == _guideGeneration) guideLoading.value = false;
    }
  }

  Future<bool> record(GuideProgram program) async {
    final data = await _toasts.attempt(
      () => _graphql.run(
        GScheduleRecordingReq(
          (b) => b.vars
            ..channel = program.channelId
            ..program = program.id
            ..start = program.start
            ..end = program.end
            ..before = 30
            ..after = 60,
        ),
      ),
      success: '“${program.title}” will be recorded',
    );
    if (data == null) return false;
    live.refetch();
    return true;
  }

  Future<void> cancelSchedule(String id) async {
    final data = await _toasts.attempt(
      () => _graphql.run(GCancelRecordingScheduleReq((b) => b.vars.id = id)),
      success: 'Recording cancelled',
    );
    if (data != null) live.refetch();
  }

  Future<void> deleteRecording(String id, {required bool deleteFile}) async {
    final data = await _toasts.attempt(
      () => _graphql.run(
        GDeleteRecordingReq(
          (b) => b.vars
            ..id = id
            ..files = deleteFile,
        ),
      ),
      success: deleteFile ? 'Recording and file deleted' : 'Recording removed',
    );
    if (data != null) live.refetch();
  }

  void dispose() {
    _poll.cancel();
    _ticker.cancel();
    _stopFollowingGuide();
    _guideGeneration++;
    live.dispose();
  }
}
