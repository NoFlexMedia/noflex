// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';
import 'dart:io';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:path_provider/path_provider.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../services/network_service.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/artwork.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../browse/work_browser.dart';
import '../player/playback_service.dart';
import 'download_models.dart';
import 'downloads_service.dart';

class DownloadsScreen extends SignalWidget {
  const DownloadsScreen({super.key, this.offline = false});

  /// Running without the server: no settings to change, nowhere else to go.
  final bool offline;

  @override
  Widget build(BuildContext context) {
    final downloads = getIt<DownloadsService>();
    final items = [...downloads.items.value]
      ..sort((a, b) => b.addedAt.compareTo(a.addedAt));
    final session = getIt<SessionService>();
    final smartOn = session.device.value?.smartDownloads ?? false;
    final gutter = context.gutter;
    final active = items.where((item) => !item.isFinished).toList();
    final finished = items.where((item) => item.isFinished).toList();
    return SafeArea(
      child: CustomScrollView(
        slivers: [
          SliverPadding(
            padding: EdgeInsets.fromLTRB(gutter, 20, gutter, 8),
            sliver: SliverToBoxAdapter(
              child: Row(
                children: [
                  Expanded(child: Text('Downloads', style: NfType.pageTitle)),
                  if (!offline)
                    NfIconButton(
                      icon: PhosphorIconsRegular.gearSix,
                      label: 'Download settings',
                      onPressed: () => getIt<NavService>().push(
                        const DownloadSettingsScreen(),
                      ),
                    ),
                ],
              ),
            ),
          ),
          SliverPadding(
            padding: EdgeInsets.symmetric(horizontal: gutter),
            sliver: SliverToBoxAdapter(
              child: Text(
                '${bytes(downloads.usedBytes.value)} on this device'
                '${smartOn ? ' · Smart Downloads on' : ''}',
                style: NfType.small,
              ),
            ),
          ),
          if (items.isEmpty)
            SliverFillRemaining(
              hasScrollBody: false,
              child: NfState(
                icon: PhosphorIconsRegular.downloadSimple,
                title: 'Nothing downloaded yet',
                message: offline
                    ? 'Titles you download will play here without a connection.'
                    : smartOn
                    ? 'Smart Downloads will fetch what you’re likely to watch next.'
                    : 'Open a film or episode and download it, or turn on Smart Downloads to have what you’re likely to watch next ready to go.',
                actionLabel: offline || smartOn
                    ? null
                    : 'Set up Smart Downloads',
                onAction: offline || smartOn
                    ? null
                    : () => getIt<NavService>().push(
                        const DownloadSettingsScreen(),
                      ),
              ),
            )
          else ...[
            if (active.isNotEmpty) ...[
              _Heading('In progress', gutter),
              _ItemList(items: active),
            ],
            if (finished.isNotEmpty) ...[
              _Heading('Ready to watch', gutter),
              _ItemList(items: finished),
            ],
            const SliverToBoxAdapter(child: SizedBox(height: 32)),
          ],
        ],
      ),
    );
  }
}

class _Heading extends StatelessWidget {
  const _Heading(this.label, this.gutter);
  final String label;
  final double gutter;

  @override
  Widget build(BuildContext context) => SliverPadding(
    padding: EdgeInsets.fromLTRB(gutter, 24, gutter, 8),
    sliver: SliverToBoxAdapter(child: NfSectionHead(label)),
  );
}

/// One column on a phone, two once there's room, split on a fold if there is one.
class _ItemList extends StatelessWidget {
  const _ItemList({required this.items});
  final List<OfflineItem> items;

  @override
  Widget build(BuildContext context) => SliverPadding(
    padding: EdgeInsets.symmetric(horizontal: context.gutter),
    sliver: SliverLayoutBuilder(
      builder: (context, constraints) {
        final columns = constraints.crossAxisExtent >= 760 ? 2 : 1;
        return SliverGrid.builder(
          gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
            crossAxisCount: columns,
            crossAxisSpacing: 12,
            mainAxisSpacing: 10,
            mainAxisExtent: 104,
          ),
          itemCount: items.length,
          itemBuilder: (context, index) => _ItemRow(item: items[index]),
        );
      },
    ),
  );
}

class _ItemRow extends SignalWidget {
  const _ItemRow({required this.item});
  final OfflineItem item;

  @override
  Widget build(BuildContext context) {
    final downloads = getIt<DownloadsService>();
    final network = getIt<NetworkService>();
    final wifiOnly =
        getIt<SessionService>().device.value?.downloadsWifiOnly ?? true;
    final waitingForWifi =
        !item.isFinished &&
        item.status == OfflineStatus.queued &&
        wifiOnly &&
        !network.unmetered.value;
    final status = switch (item.status) {
      OfflineStatus.complete => [
        bytes(item.sizeBytes),
        if (item.origin == OfflineOrigin.smart) 'Smart',
        if (item.played)
          'Watched'
        else if (item.positionMs > 0)
          timecode(item.positionMs),
      ].join(' · '),
      OfflineStatus.queued => waitingForWifi ? 'Waiting for Wi-Fi' : 'Queued',
      OfflineStatus.downloading =>
        '${(item.progress * 100).round()}% of ${bytes(item.sizeBytes)}',
      OfflineStatus.paused => 'Paused at ${(item.progress * 100).round()}%',
      OfflineStatus.failed => item.error ?? 'The download stopped.',
    };
    return NfPanel(
      padding: const EdgeInsets.all(10),
      onTap: item.isFinished
          ? () => getIt<PlaybackService>().playWork(item.work)
          : null,
      child: Row(
        children: [
          SizedBox(
            width: 56,
            height: 84,
            child: Artwork(
              work: item.work,
              localPath: downloads.localPosterPath(item),
              radius: NfRadii.s,
            ),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Text(
                  item.work.heading,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: nfText(14.5, weight: 600),
                ),
                if (item.work.isEpisode)
                  Text(
                    [
                      item.work.subtitle,
                      item.work.episodeTitle,
                    ].where((part) => part.isNotEmpty).join(' · '),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: NfType.caption,
                  ),
                const SizedBox(height: 4),
                Text(
                  status,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: nfText(
                    12.5,
                    color: item.status == OfflineStatus.failed
                        ? NfColors.danger
                        : NfColors.text2,
                  ),
                ),
                if (item.status == OfflineStatus.downloading ||
                    item.status == OfflineStatus.paused) ...[
                  const SizedBox(height: 6),
                  NfProgressBar(
                    value: item.progress.clamp(0.0, 1.0),
                    height: 3,
                  ),
                ],
              ],
            ),
          ),
          if (item.status == OfflineStatus.downloading)
            NfIconButton(
              icon: PhosphorIconsRegular.pause,
              label: 'Pause',
              size: 40,
              onPressed: () => downloads.pause(item),
            ),
          if (item.status == OfflineStatus.paused)
            NfIconButton(
              icon: PhosphorIconsRegular.play,
              label: 'Resume',
              size: 40,
              onPressed: () => downloads.resume(item),
            ),
          if (item.status == OfflineStatus.failed)
            NfIconButton(
              icon: PhosphorIconsRegular.arrowClockwise,
              label: 'Try again',
              size: 40,
              onPressed: () => downloads.retry(item, null),
            ),
          NfIconButton(
            icon: PhosphorIconsRegular.trash,
            label: 'Remove download',
            size: 40,
            iconSize: 18,
            color: NfColors.text3,
            onPressed: () async {
              final confirmed = await confirmAction(
                title: 'Remove “${item.work.heading}”?',
                message: item.origin == OfflineOrigin.smart
                    ? 'It may come back if Smart Downloads still thinks you’ll watch it.'
                    : 'It’ll need downloading again to watch offline.',
                confirmLabel: 'Remove',
                danger: true,
              );
              if (confirmed) await downloads.remove(item);
            },
          ),
        ],
      ),
    );
  }
}

class _SettingRow extends StatelessWidget {
  const _SettingRow({
    required this.title,
    this.detail,
    this.trailing,
    this.onTap,
  });

  final String title;
  final String? detail;
  final Widget? trailing;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) => InkWell(
    onTap: onTap,
    borderRadius: BorderRadius.circular(NfRadii.s),
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: 12, horizontal: 4),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title, style: nfText(15, weight: 550)),
                if (detail != null) ...[
                  const SizedBox(height: 2),
                  Text(detail!, style: NfType.small),
                ],
              ],
            ),
          ),
          if (trailing != null) ...[const SizedBox(width: 12), trailing!],
        ],
      ),
    ),
  );
}

const _budgets = [
  (null, 'No limit'),
  (5 * 1024 * 1024 * 1024, '5 GB'),
  (10 * 1024 * 1024 * 1024, '10 GB'),
  (25 * 1024 * 1024 * 1024, '25 GB'),
  (50 * 1024 * 1024 * 1024, '50 GB'),
  (100 * 1024 * 1024 * 1024, '100 GB'),
];

const _prepareQualities = [
  (null, 'Best it can play'),
  (20000000, 'Up to 20 Mbps'),
  (8000000, 'Up to 8 Mbps'),
  (4000000, 'Up to 4 Mbps'),
];

class DownloadSettingsScreen extends SignalWidget {
  const DownloadSettingsScreen({super.key});

  Future<void> _chooseFolder(DownloadsService downloads) async {
    if (Platform.isAndroid) {
      // Android's app-specific folders need no permission and include SD cards.
      final volumes =
          await getExternalStorageDirectories() ?? const <Directory>[];
      final choice = await pickOption<String>(
        title: 'Save downloads to',
        selected: downloads.targetDirectory.value ?? '',
        options: [
          const NfOption(
            '',
            'This device',
            detail: 'App storage, removed with the app',
          ),
          for (final (index, volume) in volumes.indexed)
            NfOption(
              volume.path,
              index == 0
                  ? 'Shared storage'
                  : 'SD card or USB ${index == 1 ? '' : index}',
              detail: volume.path,
            ),
        ],
      );
      if (choice != null) {
        await downloads.setTargetDirectory(choice.isEmpty ? null : choice);
      }
      return;
    }
    final path = await FilePicker.getDirectoryPath(
      dialogTitle: 'Save downloads to',
    );
    if (path != null) await downloads.setTargetDirectory(path);
  }

  @override
  Widget build(BuildContext context) {
    final downloads = getIt<DownloadsService>();
    final device = getIt<SessionService>().device.value;
    final gutter = context.gutter;
    final budget = device?.downloadBudgetBytes;
    final target = downloads.targetDirectory.value;
    final quality = device?.maxBitrateBps;
    // iOS apps have the one sandbox; there's nothing to choose between.
    final canChooseFolder = !Platform.isIOS;
    return Scaffold(
      appBar: const NfPageBar(title: 'Download settings'),
      body: device == null
          ? const NfState(
              title: 'This device isn’t registered yet',
              message: 'It registers with the server once it’s measured what it can play. Check back in a moment.',
            )
          : ListView(
              padding: EdgeInsets.fromLTRB(gutter, 8, gutter, 32),
              children: [
                Center(
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 640),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        _SettingRow(
                          title: 'Smart Downloads',
                          detail: 'Keep the next episodes of what you’re watching and what you’re likely to watch next on this device, and remove what you’ve finished.',
                          trailing: Switch(
                            value: device.smartDownloads,
                            onChanged: (value) =>
                                downloads.updateDevice(smartDownloads: value),
                          ),
                        ),
                        _SettingRow(
                          title: 'Wait for Wi-Fi',
                          detail: 'Only download on Wi-Fi or a wired connection. Transfers pause on mobile data and pick up when you’re back.',
                          trailing: Switch(
                            value: device.downloadsWifiOnly,
                            onChanged: (value) =>
                                downloads.updateDevice(wifiOnly: value),
                          ),
                        ),
                        _SettingRow(
                          title: 'Space for Smart Downloads',
                          detail: budget == null ? 'No limit' : bytes(budget),
                          trailing: const Icon(
                            PhosphorIconsRegular.caretRight,
                            size: 16,
                            color: NfColors.text3,
                          ),
                          onTap: () async {
                            const noLimit = -1;
                            final choice = await pickOption<int>(
                              title: 'Space for Smart Downloads',
                              selected: budget ?? noLimit,
                              options: [
                                for (final (value, label) in _budgets)
                                  NfOption(value ?? noLimit, label),
                              ],
                            );
                            if (choice == null) return;
                            await downloads.updateDevice(
                              budgetBytes: choice == noLimit ? null : choice,
                              clearBudget: choice == noLimit,
                            );
                          },
                        ),
                        if (canChooseFolder)
                          _SettingRow(
                            title: 'Save downloads to',
                            detail: target ?? 'This device',
                            trailing: const Icon(
                              PhosphorIconsRegular.caretRight,
                              size: 16,
                              color: NfColors.text3,
                            ),
                            onTap: () => _chooseFolder(downloads),
                          ),
                        const Divider(height: 28),
                        _SettingRow(
                          title: 'Prepare versions on the server',
                          detail: 'While it’s idle, the server encodes what you’re likely to watch into a version this device plays in hardware. Those stream and download without a live transcode.',
                          trailing: Switch(
                            value: device.smartTranscode,
                            onChanged: (value) =>
                                downloads.updateDevice(smartTranscode: value),
                          ),
                        ),
                        _SettingRow(
                          title: 'Quality of prepared versions',
                          detail: _prepareQualities
                              .firstWhere(
                                (q) => q.$1 == quality,
                                orElse: () => _prepareQualities.first,
                              )
                              .$2,
                          trailing: const Icon(
                            PhosphorIconsRegular.caretRight,
                            size: 16,
                            color: NfColors.text3,
                          ),
                          onTap: () async {
                            const best = -1;
                            final choice = await pickOption<int>(
                              title: 'Quality of prepared versions',
                              selected: quality ?? best,
                              options: [
                                for (final (value, label) in _prepareQualities)
                                  NfOption(value ?? best, label),
                              ],
                            );
                            if (choice == null) return;
                            await downloads.updateDevice(
                              maxBitrateBps: choice == best ? null : choice,
                              clearBitrate: choice == best,
                            );
                          },
                        ),
                        const SizedBox(height: 16),
                        Text(
                          Platform.isIOS
                              ? 'iOS finishes downloads in the background, but decides for itself when NoFlex may check for new ones, so Smart Downloads catches up most reliably after you’ve opened the app.'
                              : 'Downloads keep going in the background once started.',
                          style: NfType.caption,
                        ),
                      ],
                    ),
                  ),
                ),
              ],
            ),
    );
  }
}

/// The app without the server: what's downloaded, and a way back.
class OfflineShell extends StatelessWidget {
  const OfflineShell({super.key});

  @override
  Widget build(BuildContext context) => Scaffold(
    body: Column(
      children: [
        SafeArea(
          bottom: false,
          child: Container(
            color: NfColors.surface2,
            padding: const EdgeInsets.fromLTRB(16, 10, 8, 10),
            child: Row(
              children: [
                const Icon(
                  PhosphorIconsRegular.cloudSlash,
                  size: 18,
                  color: NfColors.warning,
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(
                    'Can’t reach your server. Downloads still play.',
                    style: nfText(13.5),
                  ),
                ),
                TextButton(
                  onPressed: () => unawaited(getIt<SessionService>().start()),
                  child: Text(
                    'Retry',
                    style: nfText(13.5, weight: 600, color: NfColors.accent),
                  ),
                ),
              ],
            ),
          ),
        ),
        const Expanded(child: DownloadsScreen(offline: true)),
      ],
    ),
  );
}
