// SPDX-License-Identifier: AGPL-3.0-only

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/admin_active_streams.data.gql.dart';
import '../../graphql/queries/__generated__/admin_active_streams.req.gql.dart';
import '../../graphql/queries/__generated__/admin_active_streams.var.gql.dart';
import '../../graphql/queries/__generated__/admin_indexing.data.gql.dart';
import '../../graphql/queries/__generated__/admin_indexing.req.gql.dart';
import '../../graphql/queries/__generated__/admin_indexing.var.gql.dart';
import '../../graphql/queries/__generated__/admin_transcoder.data.gql.dart';
import '../../graphql/queries/__generated__/admin_transcoder.req.gql.dart';
import '../../graphql/queries/__generated__/admin_transcoder.var.gql.dart';
import '../../graphql/query.dart';
import '../../services/graphql.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../account/account_widgets.dart';
import 'admin_controller.dart';
import 'admin_format.dart';
import 'admin_issues.dart';
import 'admin_widgets.dart';

/// The live sample's state: how fresh these numbers are.
class LiveBadge extends SignalWidget {
  const LiveBadge({super.key, required this.live});
  final LiveStatus live;

  @override
  Widget build(BuildContext context) {
    final mode = live.mode.value;
    final sampled = live.status.value?.sampledAt;
    final color = switch (mode) {
      LiveMode.live => NfColors.success,
      LiveMode.polling => NfColors.warning,
      LiveMode.offline => NfColors.danger,
      LiveMode.connecting => NfColors.text3,
    };
    return Tooltip(
      message: live.error.value ?? mode.detail,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 6),
        decoration: BoxDecoration(
          color: NfColors.surface2,
          borderRadius: BorderRadius.circular(NfRadii.pill),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 7,
              height: 7,
              decoration: BoxDecoration(color: color, shape: BoxShape.circle),
            ),
            const SizedBox(width: 8),
            Text(
              mode.label,
              style: nfText(12, weight: 600, color: NfColors.text2),
            ),
            if (sampled != null)
              Text(
                ' · ${clock(sampled)}',
                style: nfText(
                  12,
                  color: NfColors.text3,
                  features: NfType.tabular,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class OverviewSection extends StatefulWidget {
  const OverviewSection({super.key, required this.controller});
  final AdminController controller;

  @override
  State<OverviewSection> createState() => _OverviewSectionState();
}

class _OverviewSectionState extends State<OverviewSection> {
  late final Query<GAdminActiveStreamsData, GAdminActiveStreamsVars> _streams;
  late final Query<GAdminIndexingData, GAdminIndexingVars> _indexing;
  late final Query<GAdminTranscoderData, GAdminTranscoderVars> _transcoder;
  late final Poller _streamsPoll;
  late final Poller _indexingPoll;

  @override
  void initState() {
    super.initState();
    final client = getIt<GraphqlService>().client;
    _streams = Query(
      client,
      GAdminActiveStreamsReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    _indexing = Query(
      client,
      GAdminIndexingReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    _transcoder = Query(client, GAdminTranscoderReq());
    _streamsPoll = Poller(_streams.refetch, const Duration(seconds: 5));
    _indexingPoll = Poller(_indexing.refetch, const Duration(seconds: 20));
  }

  @override
  void dispose() {
    _streamsPoll.cancel();
    _indexingPoll.cancel();
    _streams.dispose();
    _indexing.dispose();
    _transcoder.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final controller = widget.controller;
      final live = controller.live;
      final status = live.status.value;
      final history = live.history.value;
      final admin = controller.admin.data.value;
      final streams = _streams.data.value?.activeStreams.toList() ?? const [];
      final transcoding = streams
          .where((s) => s.decision.route == GPlaybackRouteKind.TRANSCODE)
          .length;
      final info = _transcoder.data.value?.transcoderInfo;
      final libraries =
          _indexing.data.value?.scannerStatus.toList() ?? const [];
      void go(AdminSection section) => controller.open(section);

      final issues = status == null
          ? const <AdminIssue>[]
          : adminIssues(
              status: status,
              transcoder: info,
              libraries: libraries,
              streams: streams,
            );

      return AdminStack(
        children: [
          Wrap(
            spacing: 12,
            runSpacing: 8,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              LiveBadge(live: live),
              if (status != null)
                Text(
                  '${status.host.hostname ?? 'Server'} · ${status.host.osName ?? ''} · up ${uptime(status.server.uptimeSeconds)}',
                  style: NfType.caption,
                ),
            ],
          ),
          if (status == null)
            const Center(
              child: Padding(
                padding: EdgeInsets.all(40),
                child: CircularProgressIndicator(),
              ),
            )
          else
            AdminColumns(
              minWidth: 165,
              children: [
                StatTile(
                  icon: PhosphorIconsRegular.squaresFour,
                  label: 'CPU',
                  value: percent(status.cpu.usagePercent),
                  sub: [
                    if (status.cpu.loadAverage != null)
                      'load ${status.cpu.loadAverage!.one.toStringAsFixed(2)}',
                    '${status.cpu.logicalCores} threads',
                  ].join(' · '),
                  tone: status.cpu.usagePercent >= 85
                      ? StateTone.warn
                      : StateTone.none,
                  onTap: () => go(AdminSection.resources),
                  spark: Sparkline(
                    values: [for (final h in history) h.cpuPercent],
                    max: 100,
                    label: 'CPU usage, last minutes',
                  ),
                ),
                StatTile(
                  icon: PhosphorIconsRegular.stack,
                  label: 'Memory',
                  value: size(status.memory.usedBytes),
                  small:
                      '/ ${size(status.memory.cgroupLimitBytes ?? status.memory.totalBytes)}',
                  sub: [
                    '${percent(memoryPercent(status))} used',
                    if (status.memory.swapUsedBytes > 0)
                      'swap ${size(status.memory.swapUsedBytes)}',
                  ].join(' · '),
                  tone: memoryPercent(status) >= 90
                      ? StateTone.warn
                      : StateTone.none,
                  onTap: () => go(AdminSection.resources),
                  spark: Sparkline(
                    values: [
                      for (final h in history) h.memoryUsedBytes.toDouble(),
                    ],
                    max: status.memory.totalBytes.toDouble(),
                    color: NfColors.focus,
                    label: 'Memory used, last minutes',
                  ),
                ),
                StatTile(
                  icon: PhosphorIconsRegular.broadcast,
                  label: 'Network',
                  value: '↓ ${rate(status.networkTotal.rxBytesPerSec)}',
                  sub:
                      '↑ ${rate(status.networkTotal.txBytesPerSec)} · all interfaces',
                  onTap: () => go(AdminSection.resources),
                  spark: Sparkline(
                    values: [for (final h in history) h.rxBytesPerSec],
                    alt: [for (final h in history) h.txBytesPerSec],
                    color: NfColors.success,
                    label: 'Network receive and transmit, last minutes',
                  ),
                ),
                StatTile(
                  icon: PhosphorIconsRegular.hardDrives,
                  label: 'NoFlex process',
                  value: percent(status.process.cpuPercent),
                  small: size(status.process.rssBytes),
                  sub:
                      'pid ${status.process.pid} · ${status.process.threads ?? '?'} threads · v${status.server.version}',
                  onTap: () => go(AdminSection.resources),
                  spark: Sparkline(
                    values: [for (final h in history) h.processCpuPercent],
                    color: NfColors.warning,
                    label: 'Server process CPU, last minutes',
                  ),
                ),
              ],
            ),
          AdminColumns(
            minWidth: 165,
            children: [
              StatTile(
                icon: PhosphorIconsFill.play,
                label: 'Streams',
                value: '${streams.length}',
                small: '$transcoding transcoding',
                onTap: () => go(AdminSection.playback),
              ),
              StatTile(
                icon: PhosphorIconsRegular.arrowsClockwise,
                label: 'Scans running',
                value: '${status?.activeScans ?? '—'}',
                small: plural(libraries.length, 'library', 'libraries'),
                onTap: () => go(AdminSection.indexing),
              ),
              StatTile(
                icon: PhosphorIconsRegular.warningCircle,
                label: 'Errors',
                value: '${status?.logs.errors ?? '—'}',
                small: '${status?.logs.warnings ?? 0} warn',
                tone: (status?.logs.errors ?? 0) > 0
                    ? StateTone.bad
                    : (status?.logs.warnings ?? 0) > 0
                    ? StateTone.warn
                    : StateTone.none,
                onTap: () => go(AdminSection.logs),
              ),
              StatTile(
                icon: PhosphorIconsRegular.plug,
                label: 'Plugins',
                value: status == null
                    ? '—'
                    : '${status.plugins.loaded - status.plugins.circuitOpen}',
                small: 'of ${status?.plugins.loaded ?? 0} healthy',
                tone: (status?.plugins.circuitOpen ?? 0) > 0
                    ? StateTone.bad
                    : StateTone.none,
                onTap: () => go(AdminSection.plugins),
              ),
              StatTile(
                icon: PhosphorIconsRegular.users,
                label: 'Accounts',
                value: '${admin?.users.length ?? '—'}',
                small: '${admin?.sessions.length ?? 0} sessions',
                onTap: () => go(AdminSection.users),
              ),
              StatTile(
                icon: PhosphorIconsRegular.filmStrip,
                label: 'Transcoding',
                value: info == null ? '—' : backendLabel(info.resolvedBackend),
                small: info == null
                    ? null
                    : info.hardwareAccelerated
                    ? 'hardware'
                    : 'software',
                onTap: () => go(AdminSection.transcoding),
              ),
            ],
          ),
          AdminColumns(
            children: [
              AccountPanel(
                icon: issues.isEmpty
                    ? PhosphorIconsRegular.checkCircle
                    : PhosphorIconsRegular.warningCircle,
                title: issues.isEmpty ? 'All clear' : 'Needs attention',
                detail: issues.isEmpty
                    ? 'No errors, failing plugins, unreadable folders or full disks.'
                    : 'Problems the server can see right now, worst first.',
                child: _Issues(issues: issues, onOpen: go),
              ),
              AccountPanel(
                icon: PhosphorIconsFill.play,
                title: 'Now playing',
                detail: streams.isEmpty
                    ? 'Nobody is streaming right now.'
                    : '${streams.length} active · $transcoding transcoding · ${streams.where((s) => s.decision.hardware).length} on hardware',
                trailing: NfButton(
                  label: 'Details',
                  style: NfButtonStyle.ghost,
                  size: NfButtonSize.small,
                  onPressed: () => go(AdminSection.playback),
                ),
                child: Column(
                  children: [
                    for (final stream in streams.take(5))
                      AccountRow(
                        title:
                            stream.work?.title ??
                            (stream.channelId != null
                                ? 'Live TV'
                                : 'Unknown title'),
                        detail: [
                          stream.user.name,
                          if (stream.device != null) stream.device!,
                          'started ${relative(stream.startedAt)}',
                        ].join(' · '),
                        trailing: _streamState(stream),
                      ),
                  ],
                ),
              ),
            ],
          ),
          if (status != null)
            AccountPanel(
              icon: PhosphorIconsRegular.hardDrives,
              title: 'Filesystems',
              detail: 'Free space where NoFlex keeps its data and reads your media.',
              child: AdminColumns(
                minWidth: 260,
                gap: 18,
                children: [
                  for (final fs in filesystems(status))
                    Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Row(
                          children: [
                            Expanded(
                              child: Text(
                                fs.key,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: NfType.mono,
                              ),
                            ),
                            const SizedBox(width: 8),
                            Text(
                              '${size(fs.available)} free of ${size(fs.total)}',
                              style: NfType.caption,
                            ),
                          ],
                        ),
                        const SizedBox(height: 6),
                        Meter(
                          pct: ratio(fs.used, fs.total),
                          thin: true,
                          label: '${fs.key} used',
                        ),
                        const SizedBox(height: 6),
                        Text(
                          [
                            if (fs.fileSystem != null) fs.fileSystem!,
                            fs.places.join(', '),
                          ].join(' · '),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: NfType.caption,
                        ),
                      ],
                    ),
                ],
              ),
            ),
        ],
      );
    },
  );

  Widget _streamState(GAdminActiveStreamsData_activeStreams stream) {
    final decision = stream.decision;
    if (decision.route == GPlaybackRouteKind.DIRECT) {
      return const StatePill('Direct', tone: StateTone.ok);
    }
    final speed = stream.transcode?.speed;
    return StatePill(
      '${decision.hardware ? 'HW' : 'SW'} transcode${speed == null ? '' : ' · ${speed.toStringAsFixed(1)}×'}',
      tone: decision.hardware ? StateTone.accent : StateTone.info,
    );
  }
}

class _Issues extends StatelessWidget {
  const _Issues({required this.issues, required this.onOpen});
  final List<AdminIssue> issues;
  final ValueChanged<AdminSection> onOpen;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      for (final issue in issues.take(8))
        InkWell(
          borderRadius: BorderRadius.circular(NfRadii.s),
          onTap: () => onOpen(issue.section),
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 7),
            child: Row(
              children: [
                Container(
                  width: 30,
                  height: 30,
                  decoration: BoxDecoration(
                    color: toneColors(
                      issue.bad ? StateTone.bad : StateTone.warn,
                    ).bg,
                    borderRadius: BorderRadius.circular(9),
                  ),
                  child: Icon(
                    issue.icon,
                    size: 15,
                    color: toneColors(
                      issue.bad ? StateTone.bad : StateTone.warn,
                    ).fg,
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(issue.title, style: nfText(14, weight: 600)),
                      if (issue.detail.isNotEmpty)
                        Text(
                          issue.detail,
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                          style: NfType.caption,
                        ),
                    ],
                  ),
                ),
                const Icon(
                  PhosphorIconsRegular.caretRight,
                  size: 16,
                  color: NfColors.text3,
                ),
              ],
            ),
          ),
        ),
      if (issues.length > 8)
        Padding(
          padding: const EdgeInsets.only(top: 6),
          child: Text('+ ${issues.length - 8} more', style: NfType.caption),
        ),
    ],
  );
}
