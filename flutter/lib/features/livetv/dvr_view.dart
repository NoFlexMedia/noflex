// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import 'channel_logo.dart';
import 'live_tv_controller.dart';

/// Older schedules are history, not something to act on.
const _pastShown = 10;

const _sideBySideWidth = 900.0;

String _span(DateTime from, DateTime to) {
  final start = from.toLocal();
  final end = to.toLocal();
  final time = DateFormat.jm();
  final sameDay = DateUtils.isSameDay(start, end);
  return '${DateFormat.MMMEd().format(start)} · ${time.format(start)} – '
      '${sameDay ? time.format(end) : DateFormat.MMMd().add_jm().format(end)}';
}

class DvrView extends StatelessWidget {
  const DvrView({super.key, required this.controller});
  final LiveTvController controller;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    final scheduled = _ScheduledPanel(controller: controller);
    final recordings = _RecordingsPanel(controller: controller);
    return LayoutBuilder(
      builder: (context, constraints) => SingleChildScrollView(
        padding: EdgeInsets.fromLTRB(gutter, 6, gutter, 32),
        child: constraints.maxWidth >= _sideBySideWidth
            ? Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Expanded(child: scheduled),
                  const SizedBox(width: 16),
                  Expanded(child: recordings),
                ],
              )
            : Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [scheduled, const SizedBox(height: 16), recordings],
              ),
      ),
    );
  }
}

class _Panel extends StatelessWidget {
  const _Panel({
    required this.icon,
    required this.title,
    required this.detail,
    required this.children,
    required this.empty,
  });

  final IconData icon;
  final String title;
  final String detail;
  final List<Widget> children;
  final String empty;

  @override
  Widget build(BuildContext context) => NfPanel(
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Container(
              width: 38,
              height: 38,
              decoration: BoxDecoration(
                color: NfColors.surface2,
                borderRadius: BorderRadius.circular(NfRadii.m),
              ),
              child: Icon(icon, size: 18, color: NfColors.text2),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(title, style: NfType.sectionTitle),
                  const SizedBox(height: 2),
                  Text(detail, style: NfType.caption),
                ],
              ),
            ),
          ],
        ),
        const SizedBox(height: 10),
        if (children.isEmpty)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 8),
            child: Text(empty, style: NfType.small),
          )
        else
          ...children,
      ],
    ),
  );
}

class _Row extends StatelessWidget {
  const _Row({
    required this.channel,
    required this.heading,
    required this.detail,
    required this.badge,
    this.detailIsError = false,
    this.dimmed = false,
    this.trailing,
  });

  final LiveChannel? channel;
  final String heading;
  final String detail;
  final Widget badge;
  final bool detailIsError;
  final bool dimmed;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) => Opacity(
    opacity: dimmed ? 0.55 : 1,
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: 8),
      child: Row(
        children: [
          ChannelLogo(
            name: channel?.name ?? '?',
            logoUrl: channel?.logoUrl,
            size: 36,
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  heading,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: nfText(14.5, weight: 600),
                ),
                const SizedBox(height: 1),
                Text(
                  detail,
                  maxLines: detailIsError ? 4 : 2,
                  overflow: TextOverflow.ellipsis,
                  style: nfText(
                    12.5,
                    color: detailIsError ? NfColors.danger : NfColors.text3,
                    height: 1.35,
                  ),
                ),
                const SizedBox(height: 5),
                badge,
              ],
            ),
          ),
          ?trailing,
        ],
      ),
    ),
  );
}

class _ScheduledPanel extends SignalWidget {
  const _ScheduledPanel({required this.controller});
  final LiveTvController controller;

  Future<void> _cancel(LiveSchedule schedule, String label) async {
    final confirmed = await confirmAction(
      title: 'Cancel recording “$label”?',
      message: 'Nothing will be recorded for this slot.',
      confirmLabel: 'Cancel recording',
      danger: true,
    );
    if (confirmed) await controller.cancelSchedule(schedule.id);
  }

  @override
  Widget build(BuildContext context) {
    final schedules = controller.schedules.value;
    final upcoming = schedules
        .where((s) => s.status == GRecordingScheduleStatus.SCHEDULED)
        .toList();
    final past = schedules
        .where((s) => s.status != GRecordingScheduleStatus.SCHEDULED)
        .take(_pastShown);
    return _Panel(
      icon: PhosphorIconsRegular.calendarBlank,
      title: 'Scheduled',
      detail: upcoming.isEmpty
          ? 'Nothing queued'
          : '${upcoming.length} upcoming',
      empty: 'Pick a program in the guide and choose Record.',
      children: [
        for (final schedule in [...upcoming, ...past]) _scheduleRow(schedule),
      ],
    );
  }

  Widget _scheduleRow(LiveSchedule schedule) {
    final channel = controller.channelById.value[schedule.channelId];
    final channelName = channel?.name ?? 'Unknown channel';
    final title = controller.programById.value[schedule.programId]?.title;
    final isUpcoming = schedule.status == GRecordingScheduleStatus.SCHEDULED;
    final (label, color, background) = switch (schedule.status) {
      GRecordingScheduleStatus.SCHEDULED => (
        'Scheduled',
        NfColors.accent,
        NfColors.accentSoft,
      ),
      GRecordingScheduleStatus.TRIGGERED => (
        'Started',
        NfColors.text2,
        NfColors.surface2,
      ),
      _ => ('Cancelled', NfColors.text2, NfColors.surface2),
    };
    return _Row(
      channel: channel,
      heading: title ?? channelName,
      detail:
          '${title != null ? '$channelName · ' : ''}'
          '${_span(schedule.startAt, schedule.endAt)}',
      badge: NfBadge(label, color: color, background: background),
      dimmed: !isUpcoming,
      trailing: isUpcoming
          ? NfIconButton(
              icon: PhosphorIconsRegular.x,
              label: 'Cancel recording on $channelName',
              size: 40,
              iconSize: 16,
              color: NfColors.text2,
              onPressed: () => _cancel(schedule, title ?? channelName),
            )
          : null,
    );
  }
}

class _RecordingsPanel extends SignalWidget {
  const _RecordingsPanel({required this.controller});
  final LiveTvController controller;

  Future<void> _remove(LiveRecording recording) async {
    final answer = await getIt<NavService>().openSheet<_RemoveAnswer>(
      (_) => _RemoveSheet(sizeBytes: recording.sizeBytes),
    );
    if (answer == null) return;
    unawaited(
      controller.deleteRecording(recording.id, deleteFile: answer.deleteFile),
    );
  }

  @override
  Widget build(BuildContext context) {
    final recordings = controller.recordings.value;
    return _Panel(
      icon: PhosphorIconsFill.record,
      title: 'Recordings',
      detail: recordings.isEmpty
          ? 'None yet'
          : '${recordings.length} on this server',
      empty: 'Finished recordings land here.',
      children: [for (final recording in recordings) _recordingRow(recording)],
    );
  }

  Widget _recordingRow(LiveRecording recording) {
    final channel = controller.channelById.value[recording.channelId];
    final channelName = channel?.name ?? 'Unknown channel';
    final schedule = controller.schedules.value
        .where((s) => s.id == recording.scheduleId)
        .firstOrNull;
    final title = controller.programById.value[schedule?.programId]?.title;
    final failed = recording.status == GRecordingStatus.FAILED;
    final started = recording.startedAt;
    final completed = recording.completedAt;
    final (label, color, background) = switch (recording.status) {
      GRecordingStatus.SCHEDULED => (
        'Waiting',
        NfColors.text2,
        NfColors.surface2,
      ),
      GRecordingStatus.RUNNING => (
        'Recording',
        NfColors.danger,
        NfColors.dangerSoft,
      ),
      GRecordingStatus.COMPLETED => (
        'Ready',
        NfColors.success,
        NfColors.successSoft,
      ),
      GRecordingStatus.FAILED => (
        'Failed',
        NfColors.danger,
        NfColors.dangerSoft,
      ),
      _ => ('Cancelled', NfColors.text2, NfColors.surface2),
    };
    return _Row(
      channel: channel,
      heading: title ?? channelName,
      detail: failed
          ? recording.error ?? 'The recorder stopped unexpectedly.'
          : [
              if (title != null) channelName,
              if (started == null)
                'Not started'
              else
                DateFormat.MMMd().add_jm().format(started.toLocal()),
              if (started != null && completed != null)
                runtime(completed.difference(started).inMilliseconds),
              if ((recording.sizeBytes ?? 0) > 0) bytes(recording.sizeBytes),
            ].where((part) => part.isNotEmpty).join(' · '),
      detailIsError: failed,
      badge: NfBadge(label, color: color, background: background),
      trailing: NfIconButton(
        icon: PhosphorIconsRegular.trash,
        label: 'Remove recording',
        size: 40,
        iconSize: 17,
        color: NfColors.text3,
        onPressed: () => _remove(recording),
      ),
    );
  }
}

class _RemoveAnswer {
  const _RemoveAnswer({required this.deleteFile});
  final bool deleteFile;
}

class _RemoveSheet extends StatefulWidget {
  const _RemoveSheet({required this.sizeBytes});
  final int? sizeBytes;

  @override
  State<_RemoveSheet> createState() => _RemoveSheetState();
}

class _RemoveSheetState extends State<_RemoveSheet> {
  final _deleteFile = signal(false);

  @override
  Widget build(BuildContext context) {
    final size = widget.sizeBytes ?? 0;
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.all(22),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text('Remove this recording?', style: NfType.sectionTitle),
            const SizedBox(height: 8),
            Text('It disappears from your DVR list.', style: NfType.bodyMuted),
            const SizedBox(height: 12),
            Row(
              children: [
                Expanded(
                  child: Text(
                    size > 0
                        ? 'Also delete the file from disk (${bytes(size)})'
                        : 'Also delete the file from disk',
                    style: NfType.body,
                  ),
                ),
                const SizedBox(width: 12),
                SignalBuilder(
                  builder: (context) => Switch(
                    value: _deleteFile.value,
                    onChanged: (value) => _deleteFile.value = value,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 16),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                NfButton(
                  label: 'Cancel',
                  style: NfButtonStyle.ghost,
                  onPressed: () => Navigator.of(context).pop(),
                ),
                const SizedBox(width: 10),
                NfButton(
                  label: 'Remove',
                  style: NfButtonStyle.danger,
                  onPressed: () =>
                      Navigator.of(context)
                          .pop(_RemoveAnswer(deleteFile: _deleteFile.value)),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
