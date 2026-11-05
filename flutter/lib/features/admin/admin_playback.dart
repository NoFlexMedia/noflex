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
import '../../graphql/queries/fragments/__generated__/stream_summary_fields.data.gql.dart';
import '../../graphql/query.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_state.dart';
import '../detail/work_detail_screen.dart';
import 'admin_activity.dart';
import 'admin_format.dart';
import 'admin_widgets.dart';

typedef _Stream = GAdminActiveStreamsData_activeStreams;

const _plans = {
  'DIRECT_PLAY': 'Direct play',
  'REMUX': 'Remux',
  'TRANSCODE_AUDIO': 'Audio transcode',
  'TRANSCODE_VIDEO': 'Video transcode',
  'TRANSCODE': 'Full transcode',
};

StateTone _planTone(GPlaybackPlanKind plan) =>
    plan == GPlaybackPlanKind.DIRECT_PLAY
    ? StateTone.ok
    : plan == GPlaybackPlanKind.REMUX
    ? StateTone.info
    : StateTone.accent;

String _video(GStreamSummaryFields summary) {
  final codec = summary.videoCodec;
  if (codec == null) return '—';
  final profile = profileName(summary.videoProfile?.name);
  final level = summary.videoLevel;
  final depth = summary.bitDepth;
  return [
    codecLabel(codec.name),
    if (profile.isNotEmpty) ' $profile',
    if (level != null && level > 0) ' @$level',
    if (depth != null && depth > 0) ' · $depth-bit',
  ].join();
}

String _audio(GStreamSummaryFields summary) {
  final codec = summary.audioCodec;
  if (codec == null) return '—';
  final channels = channelsLabel(summary.audioChannels);
  return channels.isEmpty
      ? codec.toUpperCase()
      : '${codec.toUpperCase()} $channels';
}

class PlaybackSection extends StatefulWidget {
  const PlaybackSection({super.key});

  @override
  State<PlaybackSection> createState() => _PlaybackSectionState();
}

class _PlaybackSectionState extends State<PlaybackSection> {
  late final Query<GAdminActiveStreamsData, GAdminActiveStreamsVars> _streams;
  late final Poller _poll;

  @override
  void initState() {
    super.initState();
    _streams = Query(
      getIt<GraphqlService>().client,
      GAdminActiveStreamsReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    _poll = Poller(_streams.refetch, const Duration(seconds: 3));
  }

  @override
  void dispose() {
    _poll.cancel();
    _streams.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final data = _streams.data.value;
      final list = data?.activeStreams.toList() ?? const <_Stream>[];
      final direct = list
          .where((s) => s.decision.route == GPlaybackRouteKind.DIRECT)
          .length;
      final transcoding = list
          .where((s) => s.decision.route == GPlaybackRouteKind.TRANSCODE)
          .length;
      final hardware = list.where((s) => s.decision.hardware).length;
      return AdminStack(
        children: [
          Wrap(
            spacing: 8,
            runSpacing: 8,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              StatePill('${list.length} active'),
              StatePill('$direct direct', tone: StateTone.ok),
              StatePill('$transcoding transcoding', tone: StateTone.accent),
              StatePill('$hardware on hardware', tone: StateTone.info),
              Text('Refreshes every 3 s', style: NfType.caption),
            ],
          ),
          if (_streams.waiting.value)
            const NfSkeletonRows()
          else if (data == null)
            NfState.error(
              title: 'Couldn’t read active streams',
              message: _streams.error.value?.toString(),
              retry: _streams.refetch,
            )
          else if (list.isEmpty)
            const NfState(
              icon: PhosphorIconsRegular.play,
              title: 'Nothing is playing',
              message: 'Streams appear here the moment someone presses play — with the direct-play decision, the reasons behind a transcode, and ffmpeg’s live progress.',
            )
          else
            AdminColumns(
              minWidth: 480,
              children: [for (final stream in list) _StreamCard(stream)],
            ),
        ],
      );
    },
  );
}

class _StreamCard extends StatelessWidget {
  const _StreamCard(this.stream);
  final _Stream stream;

  @override
  Widget build(BuildContext context) {
    final decision = stream.decision;
    final work = stream.work;
    final job = stream.transcode;
    final runtime = work?.runtimeMs;
    final position = stream.positionMs;
    final idle =
        DateTime.now().difference(stream.lastActiveAt) >
        const Duration(minutes: 1);
    final name = stream.user.name;
    final isTranscode = decision.route == GPlaybackRouteKind.TRANSCODE;
    final title =
        work?.title ??
        (stream.channelId != null ? 'Live TV channel' : 'Unknown title');
    return NfPanel(
      padding: const EdgeInsets.all(16),
      child: AdminStack(
        children: [
          Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              AdminAvatar(
                name.length > 2
                    ? name.substring(0, 2).toUpperCase()
                    : name.toUpperCase(),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    InkWell(
                      onTap: work == null
                          ? null
                          : () => getIt<NavService>().push(
                              WorkDetailScreen(
                                workId: work.parentId ?? work.id,
                              ),
                            ),
                      child: Text(
                        work?.year == null ? title : '$title (${work!.year})',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: nfText(15, weight: 650),
                      ),
                    ),
                    const SizedBox(height: 2),
                    Text.rich(
                      TextSpan(
                        style: NfType.caption,
                        children: [
                          TextSpan(
                            text: [
                              name,
                              if (stream.device != null) stream.device!,
                              'started ${relative(stream.startedAt)}',
                            ].join(' · '),
                          ),
                          if (idle)
                            TextSpan(
                              text: ' · idle ${relative(stream.lastActiveAt)}',
                              style: nfText(12, color: NfColors.warning),
                            ),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              StatePill(
                _plans[decision.plan.name] ?? titleCase(decision.plan.name),
                tone: _planTone(decision.plan),
              ),
              if (isTranscode)
                StatePill(
                  decision.hardware ? 'Hardware' : 'Software',
                  tone: decision.hardware ? StateTone.accent : StateTone.none,
                ),
              if (decision.tonemap)
                const StatePill('Tonemap', tone: StateTone.warn),
            ],
          ),
          Row(
            children: [
              Text(
                position == null ? '—' : timecode(position),
                style: nfText(
                  12.5,
                  color: NfColors.text2,
                  features: NfType.tabular,
                ),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Meter(
                  pct: runtime != null && runtime > 0
                      ? ratio(position, runtime)
                      : 0,
                  thin: true,
                  warn: 101,
                  bad: 101,
                  label: 'Playback position',
                ),
              ),
              const SizedBox(width: 10),
              Text(
                runtime == null || runtime == 0 ? '' : timecode(runtime),
                style: nfText(
                  12.5,
                  color: NfColors.text3,
                  features: NfType.tabular,
                ),
              ),
            ],
          ),
          if (decision.reasons.isNotEmpty)
            Wrap(
              spacing: 6,
              runSpacing: 6,
              children: [
                for (final reason in decision.reasons)
                  Tooltip(
                    message: reason.code.name,
                    child: TagChip(reason.description),
                  ),
              ],
            ),
          _Comparison(decision: decision),
          if (isTranscode)
            Facts([
              Fact('Video encoder', decision.encoder ?? '—', mono: true),
              Fact('Audio encoder', decision.audioEncoder ?? '—', mono: true),
              if (job != null) ...[
                Fact(
                  'Job',
                  job.status,
                  child: StatePill(
                    job.status,
                    tone: job.status == 'running'
                        ? StateTone.ok
                        : job.status == 'failed'
                        ? StateTone.bad
                        : StateTone.none,
                  ),
                ),
                Fact(
                  'Speed',
                  job.speed == null ? '—' : '${job.speed!.toStringAsFixed(2)}×',
                  color: (job.speed ?? 1) < 1 ? NfColors.warning : null,
                ),
                Fact(
                  'Frame rate',
                  job.fps == null ? '—' : '${job.fps!.toStringAsFixed(0)} fps',
                ),
                Fact(
                  'Encoded to',
                  job.outTimeMs == null ? '—' : timecode(job.outTimeMs!),
                ),
                Fact('Job id', job.jobId, mono: true, full: true),
              ],
            ]),
          if (job?.error != null) Notice(job!.error!),
        ],
      ),
    );
  }
}

class _Comparison extends StatelessWidget {
  const _Comparison({required this.decision});
  final GAdminActiveStreamsData_activeStreams_decision decision;

  @override
  Widget build(BuildContext context) {
    final source = decision.source;
    final target = decision.target;
    final rows = <(String, String, String)>[
      ('Container', source.container ?? '—', target.container ?? '—'),
      (
        'Video',
        _video(source),
        decision.encoder == 'copy'
            ? '${_video(target)} (copy)'
            : _video(target),
      ),
      (
        'Resolution',
        resolutionLabel(source.width, source.height),
        resolutionLabel(target.width, target.height),
      ),
      (
        'Range',
        rangeLabel(source.videoRange?.name),
        decision.tonemap
            ? '${rangeLabel(target.videoRange?.name)} (tonemapped)'
            : rangeLabel(target.videoRange?.name),
      ),
      (
        'Audio',
        _audio(source),
        decision.audioEncoder == 'copy'
            ? '${_audio(target)} (copy)'
            : _audio(target),
      ),
      ('Bitrate', bitrate(source.bitrateBps), bitrate(target.bitrateBps)),
    ];
    return AdminTable(
      minWidth: 0,
      columns: const [
        AdminCol(''),
        AdminCol('Source', flex: true),
        AdminCol('Delivered', flex: true),
      ],
      rows: [
        for (final (label, from, to) in rows)
          [
            Text(
              label.toUpperCase(),
              style: nfText(
                10,
                weight: 600,
                color: NfColors.text3,
                letterSpacing: 1,
              ),
            ),
            cellText(from),
            cellText(
              to,
              strong: from != to && to != '—',
              color: from != to && to != '—' ? NfColors.accent : null,
            ),
          ],
      ],
    );
  }
}
