// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/admin_play_chart.data.gql.dart';
import '../../graphql/queries/__generated__/admin_play_chart.req.gql.dart';
import '../../graphql/queries/__generated__/admin_play_chart.var.gql.dart';
import '../../graphql/queries/__generated__/admin_play_sessions.data.gql.dart';
import '../../graphql/queries/__generated__/admin_play_sessions.req.gql.dart';
import '../../graphql/queries/__generated__/admin_top_played.data.gql.dart';
import '../../graphql/queries/__generated__/admin_top_played.req.gql.dart';
import '../../graphql/queries/__generated__/admin_top_played.var.gql.dart';
import '../../graphql/queries/__generated__/admin_top_users.data.gql.dart';
import '../../graphql/queries/__generated__/admin_top_users.req.gql.dart';
import '../../graphql/queries/__generated__/admin_top_users.var.gql.dart';
import '../../graphql/query.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../utils/images.dart';
import '../../widgets/artwork.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../account/account_widgets.dart';
import '../detail/work_detail_screen.dart';
import 'admin_controller.dart';
import 'admin_format.dart';
import 'admin_widgets.dart';

final _ranges = <(GStatsRange, String, String)>[
  (GStatsRange.LAST_24_HOURS, '24h', 'the last 24 hours'),
  (GStatsRange.LAST_7_DAYS, '7d', 'the last 7 days'),
  (GStatsRange.LAST_30_DAYS, '30d', 'the last 30 days'),
  (GStatsRange.LAST_90_DAYS, '90d', 'the last 90 days'),
  (GStatsRange.LAST_YEAR, '1y', 'the last year'),
  (GStatsRange.ALL_TIME, 'All', 'all time'),
];

String _rangeName(GStatsRange range) =>
    _ranges.firstWhere((item) => item.$1 == range).$3;

/// The device's current offset, which the server aligns buckets to.
int _utcOffset() => DateTime.now().timeZoneOffset.inMinutes;

IconData libraryKindIcon(GLibraryKind? kind) => switch (kind) {
  GLibraryKind.MOVIES => PhosphorIconsRegular.filmStrip,
  GLibraryKind.SHOWS => PhosphorIconsRegular.television,
  GLibraryKind.MUSIC => PhosphorIconsRegular.musicNotes,
  GLibraryKind.AUDIOBOOKS => PhosphorIconsRegular.bookOpen,
  GLibraryKind.PHOTOS => PhosphorIconsRegular.image,
  GLibraryKind.MIXED => PhosphorIconsRegular.stack,
  _ => PhosphorIconsRegular.folder,
};

class ActivitySection extends StatefulWidget {
  const ActivitySection({super.key, required this.controller});
  final AdminController controller;

  @override
  State<ActivitySection> createState() => _ActivitySectionState();
}

class _ActivitySectionState extends State<ActivitySection> {
  final _range = signal(GStatsRange.LAST_30_DAYS);
  final _library = signal<String?>(null);
  final _user = signal<String?>(null);
  final _hours = signal(false);

  final _offset = _utcOffset();
  final _chart = signal<Query<GAdminPlayChartData, GAdminPlayChartVars>?>(null);
  final _topUsers = signal<Query<GAdminTopUsersData, GAdminTopUsersVars>?>(
    null,
  );
  final _topPlayed = signal<Query<GAdminTopPlayedData, GAdminTopPlayedVars>?>(
    null,
  );
  late final void Function() _stopRange;
  late final void Function() _stopChart;

  @override
  void initState() {
    super.initState();
    final client = getIt<GraphqlService>().client;
    _stopRange = effect(() {
      final range = _range.value;
      untracked(() {
        _topUsers.value?.dispose();
        _topPlayed.value?.dispose();
        _topUsers.value = Query(
          client,
          GAdminTopUsersReq(
            (b) => b.vars
              ..range = range
              ..offset = _offset,
          ),
        );
        _topPlayed.value = Query(
          client,
          GAdminTopPlayedReq(
            (b) => b.vars
              ..range = range
              ..offset = _offset,
          ),
        );
      });
    });
    _stopChart = effect(() {
      final range = _range.value;
      final library = _library.value;
      final user = _user.value;
      untracked(() {
        _chart.value?.dispose();
        _chart.value = Query(
          client,
          GAdminPlayChartReq(
            (b) => b.vars
              ..range = range
              ..offset = _offset
              ..library = library
              ..user = user,
          ),
        );
      });
    });
  }

  @override
  void dispose() {
    _stopRange();
    _stopChart();
    _chart.value?.dispose();
    _topUsers.value?.dispose();
    _topPlayed.value?.dispose();
    super.dispose();
  }

  void _openSessions(
    GAdminTopUsersData_topUsers_user user, {
    GAdminTopUsersData_topUsers_libraries_library? library,
  }) {
    final range = _range.value;
    unawaited(
      getIt<NavService>().openSheet<void>(
        (context) => _PlaySessionsSheet(
          userId: user.id,
          userName: user.name,
          libraryId: library?.id,
          libraryName: library?.name,
          range: range,
          offset: _offset,
        ),
        wide: true,
      ),
    );
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final admin = widget.controller.admin.data.value;
      final users = admin?.users.toList() ?? const [];
      final libraries = admin?.libraries.toList() ?? const [];
      final leaders = _topUsers.value;
      final played = _topPlayed.value;
      final chart = _chart.value;
      final rows = leaders?.data.value?.topUsers.toList() ?? const [];
      final groups = played?.data.value?.topPlayed.toList() ?? const [];
      final plays = rows.fold<int>(0, (sum, row) => sum + row.plays);
      final time = rows.fold<int>(0, (sum, row) => sum + row.watchedMs);
      final rangeName = _rangeName(_range.value);
      final busiest = groups.isEmpty ? null : groups.first;
      return AdminStack(
        children: [
          Wrap(
            spacing: 12,
            runSpacing: 8,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              AdminSegments<GStatsRange>(
                items: [for (final item in _ranges) (item.$1, item.$2)],
                value: _range.value,
                onChanged: (value) => _range.value = value,
              ),
              Text('Times shown in your timezone', style: NfType.caption),
            ],
          ),
          AdminColumns(
            minWidth: 165,
            children: [
              StatTile(
                icon: PhosphorIconsFill.play,
                label: 'Plays',
                value: compact(plays),
                sub: 'over $rangeName',
              ),
              StatTile(
                icon: PhosphorIconsRegular.clock,
                label: 'Watched',
                value: watched(time),
                sub: 'wall-clock playing time',
              ),
              StatTile(
                icon: PhosphorIconsRegular.users,
                label: 'Viewers',
                value: '${rows.length}',
                sub: 'of ${plural(users.length, 'account')}',
              ),
              StatTile(
                icon: PhosphorIconsRegular.sparkle,
                label: 'Busiest library',
                value: busiest?.library?.name ?? '—',
                sub: busiest == null
                    ? 'no plays yet'
                    : '${busiest.plays} plays',
              ),
            ],
          ),
          _ChartPanel(
            chart: chart,
            libraries: [for (final item in libraries) (item.id, item.name)],
            users: [for (final item in users) (item.id, item.name)],
            library: _library,
            user: _user,
            hours: _hours,
            rangeName: rangeName,
          ),
          AdminColumns(
            children: [
              AccountPanel(
                icon: PhosphorIconsRegular.users,
                title: 'Top users',
                detail: 'Ranked by time watched. Select a library to see that user’s sessions.',
                child: _TopUsers(
                  query: leaders,
                  rangeName: rangeName,
                  onOpen: _openSessions,
                ),
              ),
              AccountPanel(
                icon: PhosphorIconsRegular.trendUp,
                title: 'Most played',
                detail: 'Per library. Episodes count toward their series, tracks toward their artist.',
                child: _MostPlayed(query: played),
              ),
            ],
          ),
        ],
      );
    },
  );
}

class _ChartPanel extends SignalWidget {
  const _ChartPanel({
    required this.chart,
    required this.libraries,
    required this.users,
    required this.library,
    required this.user,
    required this.hours,
    required this.rangeName,
  });

  final Query<GAdminPlayChartData, GAdminPlayChartVars>? chart;
  final List<(String, String)> libraries;
  final List<(String, String)> users;
  final Signal<String?> library;
  final Signal<String?> user;
  final Signal<bool> hours;
  final String rangeName;

  @override
  Widget build(BuildContext context) {
    final data = chart?.data.value?.playHistoryChart;
    final filtered = library.value != null || user.value != null;
    final showHours = hours.value;
    final points = [
      if (data != null)
        for (final point in data.points)
          (start: point.start, plays: point.plays, watchedMs: point.watchedMs),
    ];
    final bucket = data?.bucket.name ?? 'DAY';
    final error = chart?.error.value;
    return NfPanel(
      padding: const EdgeInsets.all(18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('Play history', style: nfText(15.5, weight: 650)),
          const SizedBox(height: 2),
          Text(
            '${showHours ? 'Hours watched' : 'Plays'} per ${bucket.toLowerCase()}${filtered ? ' · filtered' : ''}',
            style: NfType.caption,
          ),
          const SizedBox(height: 12),
          Wrap(
            spacing: 8,
            runSpacing: 8,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              SizedBox(
                width: 170,
                child: AdminSelect<String>(
                  placeholder: 'Library',
                  options: [
                    const NfOption('', 'All libraries'),
                    for (final (id, name) in libraries) NfOption(id, name),
                  ],
                  value: library.value ?? '',
                  onChanged: (value) =>
                      library.value = value.isEmpty ? null : value,
                ),
              ),
              SizedBox(
                width: 170,
                child: AdminSelect<String>(
                  placeholder: 'User',
                  options: [
                    const NfOption('', 'All users'),
                    for (final (id, name) in users) NfOption(id, name),
                  ],
                  value: user.value ?? '',
                  onChanged: (value) =>
                      user.value = value.isEmpty ? null : value,
                ),
              ),
              AdminSegments<bool>(
                items: const [(false, 'Plays'), (true, 'Hours')],
                value: showHours,
                onChanged: (value) => hours.value = value,
              ),
            ],
          ),
          const SizedBox(height: 18),
          if (error != null && points.isEmpty)
            NfState.error(
              title: 'Couldn’t draw the chart',
              message: error.toString(),
              retry: () => chart?.refetch(),
            )
          else if (points.isEmpty)
            const SizedBox(
              height: 220,
              child: Center(child: CircularProgressIndicator()),
            )
          else
            Stack(
              alignment: Alignment.center,
              children: [
                PlayChart(
                  points: points,
                  bucket: bucket,
                  showHours: showHours,
                  label: 'Play history over $rangeName',
                ),
                if (data?.totalPlays == 0)
                  IgnorePointer(
                    child: Text(
                      'Nothing played ${filtered ? 'matching these filters ' : ''}over $rangeName.',
                      textAlign: TextAlign.center,
                      style: NfType.small,
                    ),
                  ),
              ],
            ),
        ],
      ),
    );
  }
}

String _initials(String? name) {
  final parts = (name ?? '')
      .split(RegExp(r'[\s._-]+'))
      .where((part) => part.isNotEmpty)
      .take(2);
  final letters = parts.map((part) => part[0].toUpperCase()).join();
  return letters.isEmpty ? '?' : letters;
}

class AdminAvatar extends StatelessWidget {
  const AdminAvatar(this.text, {super.key, this.size = 32});
  final String text;
  final double size;

  @override
  Widget build(BuildContext context) => Container(
    width: size,
    height: size,
    alignment: Alignment.center,
    decoration: const BoxDecoration(
      color: NfColors.surface3,
      shape: BoxShape.circle,
    ),
    child: Text(
      text,
      style: nfText(size * 0.36, weight: 700, color: NfColors.text2),
    ),
  );
}

class _TopUsers extends SignalWidget {
  const _TopUsers({
    required this.query,
    required this.rangeName,
    required this.onOpen,
  });

  final Query<GAdminTopUsersData, GAdminTopUsersVars>? query;
  final String rangeName;
  final void Function(
    GAdminTopUsersData_topUsers_user, {
    GAdminTopUsersData_topUsers_libraries_library? library,
  })
  onOpen;

  @override
  Widget build(BuildContext context) {
    final current = query;
    if (current == null || current.waiting.value) {
      return const Padding(
        padding: EdgeInsets.all(24),
        child: Center(child: CircularProgressIndicator()),
      );
    }
    final rows = current.data.value?.topUsers.toList() ?? const [];
    if (rows.isEmpty) {
      if (current.error.value != null) {
        return NfState.error(
          title: 'Couldn’t rank users',
          message: current.error.value.toString(),
          retry: current.refetch,
        );
      }
      return NfState(
        icon: PhosphorIconsRegular.users,
        title: 'No viewers yet',
        message: 'Users appear here once they play something over $rangeName.',
      );
    }
    final top = rows
        .map((row) => row.watchedMs)
        .fold(1, (a, b) => a > b ? a : b);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final (index, row) in rows.indexed)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 8),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                InkWell(
                  borderRadius: BorderRadius.circular(NfRadii.s),
                  onTap: row.user == null ? null : () => onOpen(row.user!),
                  child: Row(
                    children: [
                      SizedBox(
                        width: 18,
                        child: Text(
                          '${index + 1}',
                          textAlign: TextAlign.center,
                          style: nfText(13, weight: 700, color: NfColors.text3),
                        ),
                      ),
                      const SizedBox(width: 10),
                      AdminAvatar(_initials(row.user?.name)),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              row.user?.name ?? 'Deleted user',
                              style: nfText(15, weight: 650),
                            ),
                            Text(
                              'last played ${relative(row.lastPlayedAt)}',
                              style: NfType.caption,
                            ),
                          ],
                        ),
                      ),
                      Column(
                        crossAxisAlignment: CrossAxisAlignment.end,
                        children: [
                          Text(
                            watched(row.watchedMs),
                            style: nfText(15, weight: 650),
                          ),
                          Text(
                            plural(row.plays, 'play'),
                            style: NfType.caption,
                          ),
                        ],
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 8),
                Padding(
                  padding: const EdgeInsets.only(left: 28),
                  child: NfProgressBar(value: row.watchedMs / top),
                ),
                for (final entry in row.libraries)
                  Padding(
                    padding: const EdgeInsets.only(left: 22, top: 2),
                    child: InkWell(
                      borderRadius: BorderRadius.circular(NfRadii.s),
                      onTap: row.user == null
                          ? null
                          : () => onOpen(row.user!, library: entry.library),
                      child: Padding(
                        padding: const EdgeInsets.symmetric(
                          horizontal: 6,
                          vertical: 6,
                        ),
                        child: Row(
                          children: [
                            Icon(
                              libraryKindIcon(entry.library?.kind),
                              size: 14,
                              color: NfColors.text2,
                            ),
                            const SizedBox(width: 8),
                            Expanded(
                              child: Text(
                                entry.library?.name ?? 'Removed library',
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: nfText(13, color: NfColors.text2),
                              ),
                            ),
                            const SizedBox(width: 8),
                            Text(
                              '${watched(entry.watchedMs)} · ${entry.plays}',
                              style: NfType.caption,
                            ),
                            const SizedBox(width: 4),
                            const Icon(
                              PhosphorIconsRegular.caretRight,
                              size: 14,
                              color: NfColors.text3,
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

class _MostPlayed extends SignalWidget {
  const _MostPlayed({required this.query});
  final Query<GAdminTopPlayedData, GAdminTopPlayedVars>? query;

  @override
  Widget build(BuildContext context) {
    final current = query;
    if (current == null || current.waiting.value) {
      return const Padding(
        padding: EdgeInsets.all(24),
        child: Center(child: CircularProgressIndicator()),
      );
    }
    final groups = current.data.value?.topPlayed.toList() ?? const [];
    if (groups.isEmpty) {
      if (current.error.value != null) {
        return NfState.error(
          title: 'Couldn’t rank titles',
          message: current.error.value.toString(),
          retry: current.refetch,
        );
      }
      return const NfState(
        icon: PhosphorIconsRegular.filmStrip,
        title: 'Nothing played yet',
        message: 'The most played titles in each library show up here.',
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final group in groups) ...[
          Padding(
            padding: const EdgeInsets.only(top: 6, bottom: 8),
            child: Row(
              children: [
                Icon(
                  libraryKindIcon(group.library?.kind),
                  size: 15,
                  color: NfColors.text2,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    group.library?.name ?? 'Removed library',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: nfText(14, weight: 600),
                  ),
                ),
                Text(
                  '${titleCase(group.library?.kind.name ?? '')} · ${group.plays} plays',
                  style: NfType.caption,
                ),
              ],
            ),
          ),
          SizedBox(
            height: 216,
            child: ListView.separated(
              scrollDirection: Axis.horizontal,
              itemCount: group.items.length,
              separatorBuilder: (_, _) => const SizedBox(width: 14),
              itemBuilder: (context, index) {
                final item = group.items[index];
                final work = item.work;
                return SizedBox(
                  width: 96,
                  child: InkWell(
                    borderRadius: BorderRadius.circular(NfRadii.s),
                    onTap: work == null
                        ? null
                        : () => getIt<NavService>().push(
                            WorkDetailScreen(
                              workId: work.id,
                              seed: WorkSummary.fromCard(work),
                            ),
                          ),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        SizedBox(
                          height: 144,
                          child: Stack(
                            fit: StackFit.expand,
                            children: [
                              if (work != null)
                                Artwork(
                                  work: WorkSummary.fromCard(work),
                                  radius: NfRadii.s,
                                )
                              else
                                const NfSkeleton(radius: NfRadii.s),
                              Positioned(
                                left: 6,
                                bottom: 6,
                                child: Container(
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: 6,
                                    vertical: 2,
                                  ),
                                  decoration: BoxDecoration(
                                    color: NfColors.bgOverlay,
                                    borderRadius: BorderRadius.circular(
                                      NfRadii.xs,
                                    ),
                                  ),
                                  child: Text(
                                    '${index + 1}',
                                    style: nfText(12, weight: 750),
                                  ),
                                ),
                              ),
                            ],
                          ),
                        ),
                        const SizedBox(height: 6),
                        Text(
                          work?.title ?? 'Removed title',
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                          style: nfText(13, weight: 600, height: 1.25),
                        ),
                        Text(
                          '${plural(item.plays, 'play')} · ${plural(item.users, 'viewer')}',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: nfText(11, color: NfColors.text3),
                        ),
                      ],
                    ),
                  ),
                );
              },
            ),
          ),
        ],
      ],
    );
  }
}

typedef _PlayRow = GAdminPlaySessionsData_playSessions_nodes;

class _PlaySessionsSheet extends StatefulWidget {
  const _PlaySessionsSheet({
    required this.userId,
    required this.userName,
    required this.libraryId,
    required this.libraryName,
    required this.range,
    required this.offset,
  });

  final String userId;
  final String userName;
  final String? libraryId;
  final String? libraryName;
  final GStatsRange range;
  final int offset;

  @override
  State<_PlaySessionsSheet> createState() => _PlaySessionsSheetState();
}

class _PlaySessionsSheetState extends State<_PlaySessionsSheet> {
  late final Pager<_PlayRow> _pager;

  @override
  void initState() {
    super.initState();
    _pager = Pager((after) async {
      final data = await getIt<GraphqlService>().run(
        GAdminPlaySessionsReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NoCache
            ..vars.range = widget.range
            ..vars.user = widget.userId
            ..vars.library = widget.libraryId
            ..vars.offset = widget.offset
            ..vars.after = after,
        ),
      );
      final info = data.playSessions.pageInfo;
      return (
        data.playSessions.nodes.toList(),
        info.hasNextPage ? info.endCursor : null,
      );
    });
    unawaited(_pager.refresh());
  }

  @override
  Widget build(BuildContext context) => AdminSheet(
    title: widget.libraryName == null
        ? widget.userName
        : '${widget.userName} · ${widget.libraryName}',
    description:
        'Viewing sessions over ${_rangeName(widget.range)}, newest first.',
    footer: [
      NfButton(
        label: 'Close',
        style: NfButtonStyle.ghost,
        onPressed: () => Navigator.of(context).pop(),
      ),
    ],
    child: SignalBuilder(
      builder: (context) {
        final rows = _pager.items.value;
        if (_pager.waiting.value) {
          return const Padding(
            padding: EdgeInsets.all(32),
            child: Center(child: CircularProgressIndicator()),
          );
        }
        if (rows.isEmpty) {
          if (_pager.error.value != null) {
            return NfState.error(
              title: 'Couldn’t load the sessions',
              message: _pager.error.value.toString(),
              retry: () => unawaited(_pager.refresh()),
            );
          }
          return const NfState(
            icon: PhosphorIconsRegular.clockCounterClockwise,
            title: 'No sessions',
            message: 'Nothing was played here in this period.',
          );
        }
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final row in rows) _PlayRowView(row: row),
            if (_pager.hasMore.value)
              Padding(
                padding: const EdgeInsets.only(top: 12),
                child: Center(
                  child: NfButton(
                    label: 'Load older sessions',
                    size: NfButtonSize.small,
                    busy: _pager.loading.value,
                    onPressed: () => unawaited(_pager.loadMore()),
                  ),
                ),
              ),
          ],
        );
      },
    ),
  );
}

class _PlayRowView extends StatelessWidget {
  const _PlayRowView({required this.row});
  final _PlayRow row;

  @override
  Widget build(BuildContext context) {
    final work = row.work;
    final title = work == null
        ? 'Removed title'
        : playedTitle(
            kind: work.kind.name,
            title: work.title,
            parentIndex: work.parentIndex,
            indexNumber: work.indexNumber,
          );
    final root = row.rootWork;
    final runtimeMs = work?.runtimeMs;
    final live =
        !row.ended &&
        DateTime.now().difference(row.lastSeenAt) < const Duration(minutes: 2);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Row(
        children: [
          SizedBox(
            width: 96,
            height: 54,
            child: work == null
                ? const NfSkeleton(radius: NfRadii.s)
                : Artwork(
                    work: WorkSummary.fromCard(work),
                    kind: ArtKind.thumb,
                    radius: NfRadii.s,
                  ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  root != null && root.id != work?.id
                      ? '${root.title} — $title'
                      : title,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: nfText(14, weight: 600),
                ),
                const SizedBox(height: 2),
                Text(
                  [
                    relative(row.startedAt),
                    '${watched(row.watchedMs)} watched',
                    if (row.device != null) row.device!,
                  ].join(' · '),
                  style: NfType.caption,
                ),
                if (runtimeMs != null && runtimeMs > 0) ...[
                  const SizedBox(height: 6),
                  SizedBox(
                    width: 200,
                    child: NfProgressBar(
                      value: (row.positionMs / runtimeMs).clamp(0.0, 1.0),
                      height: 3,
                    ),
                  ),
                ],
              ],
            ),
          ),
          if (live) ...[
            const SizedBox(width: 8),
            const StatePill('Live', tone: StateTone.accent),
          ],
        ],
      ),
    );
  }
}
