// SPDX-License-Identifier: AGPL-3.0-only
//
// Files the server packaged for this account. A finished one comes onto the
// device from here. Downloads files everything under its work, so one whose work
// the viewer can no longer see is left to the web, which can save it without.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../models/work.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../downloads/download_models.dart';
import '../downloads/downloads_service.dart';
import 'account_controller.dart';
import 'account_widgets.dart';

class ExportsSection extends StatefulWidget {
  const ExportsSection({super.key, required this.controller});
  final AccountController controller;

  @override
  State<ExportsSection> createState() => _ExportsSectionState();
}

class _ExportsSectionState extends State<ExportsSection> {
  AccountController get controller => widget.controller;

  @override
  void initState() {
    super.initState();
    unawaited(controller.loadLinkedExports());
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(builder: _build);

  Widget _build(BuildContext context) {
    final jobs = controller.exportJobs.value;
    final onDevice = {
      for (final item in getIt<DownloadsService>().items.value)
        item.work.id: item.status,
    };
    if (jobs.isEmpty) {
      return const AccountColumn(
        children: [
          NfState(
            icon: PhosphorIconsRegular.export,
            title: 'No offline copies',
            message: 'Open a film or episode, choose ••• → Make an offline copy, and it’ll be prepared here.',
          ),
        ],
      );
    }
    return AccountColumn(
      children: [
        for (final entry in jobs)
          _ExportRow(
            job: entry.job,
            serverName: entry.server?.name,
            local: onDevice[entry.job.work?.id],
            onCancel: () => unawaited(_cancel(entry)),
            onRemove: () => unawaited(_remove(entry)),
            onSave: () => unawaited(_save(entry)),
          ),
      ],
    );
  }

  Future<void> _remove(ExportEntry entry) async {
    final confirmed = await confirmAction(
      title: 'Remove this offline copy?',
      message: 'The server deletes its file once nobody else needs it. You can always make another.',
      confirmLabel: 'Remove',
      danger: true,
    );
    if (confirmed) await controller.removeExport(entry);
  }

  Future<void> _cancel(ExportEntry entry) async {
    final confirmed = await confirmAction(
      title: 'Cancel this offline copy?',
      message: 'Any partial output is discarded.',
      confirmLabel: 'Cancel copy',
      danger: true,
    );
    if (confirmed) await controller.cancelExport(entry);
  }

  Future<void> _save(ExportEntry entry) async {
    final job = entry.job;
    final work = job.work;
    if (work == null) return;
    await getIt<ToastService>().attempt(
      () => getIt<DownloadsService>().fetchExport(
        jobId: job.id,
        work: WorkSummary.fromCard(work, server: entry.server),
      ),
      success: 'Saving to this device. Follow it under Downloads.',
    );
  }
}

class _ExportRow extends StatelessWidget {
  const _ExportRow({
    required this.job,
    required this.serverName,
    required this.local,
    required this.onCancel,
    required this.onRemove,
    required this.onSave,
  });
  final ExportJob job;

  /// The linked server it's on.
  final String? serverName;

  /// Where its title stands in Downloads, if it's there.
  final OfflineStatus? local;
  final VoidCallback onCancel;
  final VoidCallback onRemove;
  final VoidCallback onSave;

  @override
  Widget build(BuildContext context) {
    final (label, color, background) = switch (job.status) {
      GExportJobStatus.QUEUED => ('Queued', NfColors.text2, NfColors.surface2),
      GExportJobStatus.RUNNING => (
        'Encoding',
        NfColors.accent,
        NfColors.accentSoft,
      ),
      GExportJobStatus.CANCEL_REQUESTED => (
        'Cancelling',
        NfColors.text2,
        NfColors.surface2,
      ),
      GExportJobStatus.COMPLETED => (
        'Ready',
        NfColors.success,
        NfColors.successSoft,
      ),
      GExportJobStatus.FAILED => (
        'Failed',
        NfColors.danger,
        NfColors.dangerSoft,
      ),
      _ => ('Cancelled', NfColors.text2, NfColors.surface2),
    };
    final failed = job.status == GExportJobStatus.FAILED;
    final completed = job.status == GExportJobStatus.COMPLETED;
    final queued = job.status == GExportJobStatus.QUEUED;
    final finished = job.finishedAt;
    final work = job.work;
    final canCancel = queued || job.status == GExportJobStatus.RUNNING;
    return NfPanel(
      padding: const EdgeInsets.all(14),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          AccountGlyph(
            icon: failed
                ? PhosphorIconsRegular.warningCircle
                : completed
                ? PhosphorIconsRegular.check
                : PhosphorIconsRegular.export,
            color: color,
            background: background,
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Wrap(
                  spacing: 8,
                  runSpacing: 4,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                    Text(
                      work == null
                          ? '${job.format.name} export'
                          : WorkSummary.fromCard(work).heading,
                      style: nfText(14.5, weight: 600),
                    ),
                    NfBadge(label, color: color, background: background),
                  ],
                ),
                const SizedBox(height: 3),
                Text(
                  failed
                      ? job.error ?? 'The encoder stopped unexpectedly.'
                      : [
                          ?serverName,
                          if (work != null) job.format.name,
                          'Requested ${ago(job.createdAt)}',
                          if (finished != null)
                            'finished ${formatDate(finished, time: true)}',
                        ].join(' · '),
                  style: nfText(
                    12.5,
                    height: 1.35,
                    color: failed ? NfColors.danger : NfColors.text3,
                  ),
                ),
                if (completed && work == null) ...[
                  const SizedBox(height: 3),
                  Text(
                    'Save it from NoFlex on the web, under Account → Offline copies.',
                    style: NfType.caption,
                  ),
                ] else if (completed && local == OfflineStatus.complete) ...[
                  const SizedBox(height: 3),
                  Text('On this device.', style: NfType.caption),
                ] else if (completed &&
                    local != null &&
                    local != OfflineStatus.failed) ...[
                  const SizedBox(height: 3),
                  Text(
                    'Saving to this device. Follow it under Downloads.',
                    style: NfType.caption,
                  ),
                ] else if (completed) ...[
                  const SizedBox(height: 10),
                  NfButton(
                    label: 'Save to this device',
                    size: NfButtonSize.small,
                    icon: const Icon(PhosphorIconsRegular.downloadSimple),
                    onPressed: onSave,
                  ),
                ],
                if (isActiveExport(job)) ...[
                  const SizedBox(height: 8),
                  Row(
                    children: [
                      Expanded(
                        child: NfProgressBar(
                          value: queued ? null : job.progress.clamp(0.0, 1.0),
                        ),
                      ),
                      const SizedBox(width: 10),
                      Text(
                        queued ? 'Waiting' : '${(job.progress * 100).round()}%',
                        style: nfText(
                          12.5,
                          weight: 600,
                          color: NfColors.text2,
                          features: NfType.tabular,
                        ),
                      ),
                    ],
                  ),
                ],
              ],
            ),
          ),
          if (canCancel) ...[
            const SizedBox(width: 8),
            NfButton(
              label: 'Cancel',
              style: NfButtonStyle.ghost,
              size: NfButtonSize.small,
              onPressed: onCancel,
            ),
          ] else if (!isActiveExport(job)) ...[
            const SizedBox(width: 8),
            NfButton(
              label: 'Remove',
              semanticLabel: 'Remove ${work?.title ?? 'this offline copy'}',
              style: NfButtonStyle.ghost,
              size: NfButtonSize.small,
              onPressed: onRemove,
            ),
          ],
        ],
      ),
    );
  }
}
