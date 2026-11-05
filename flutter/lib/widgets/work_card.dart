// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../di.dart';
import '../features/detail/work_detail_screen.dart';
import '../features/player/playback_service.dart';
import '../layout/window.dart';
import '../models/work.dart';
import '../services/nav_service.dart';
import '../services/watch_state.dart';
import '../theme.dart';
import '../utils/format.dart';
import '../utils/images.dart';
import 'artwork.dart';

void openWork(WorkSummary work) => getIt<NavService>().push(
  WorkDetailScreen(workId: work.id, server: work.server, seed: work),
);

/// A poster, or a wide thumbnail for shelves like Continue Watching. Shows
/// progress and watched state as they change on this device.
class WorkCard extends StatelessWidget {
  const WorkCard({super.key, required this.work, this.wide = false, this.meta});

  final WorkSummary work;
  final bool wide;
  final String? meta;

  @override
  Widget build(BuildContext context) {
    final line =
        meta ??
        [
          work.subtitle,
          work.episodeTitle,
          if (!work.isEpisode) titleCase(work.kind),
          ?work.server?.name,
        ].where((part) => part.isNotEmpty).join(' · ');
    final frame = AspectRatio(
      aspectRatio: wide ? 16 / 9 : 2 / 3,
      child: Stack(
        fit: StackFit.expand,
        children: [
          Artwork(work: work, kind: wide ? ArtKind.thumb : ArtKind.poster),
          if (wide)
            Positioned.fill(
              child: DecoratedBox(
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(NfRadii.m),
                  gradient: const LinearGradient(
                    begin: Alignment.topCenter,
                    end: Alignment.bottomCenter,
                    stops: [0.45, 1],
                    colors: [Colors.transparent, Color(0xCC000000)],
                  ),
                ),
              ),
            ),
          if (wide)
            Positioned(
              left: 12,
              right: 12,
              bottom: 12,
              child: _Caption(heading: work.heading, line: line),
            ),
          _WatchOverlay(work: work),
          DecoratedBox(
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(NfRadii.m),
              border: Border.all(color: NfColors.line),
            ),
          ),
          if (!work.isContainer)
            Positioned(right: 8, top: 8, child: _QuickPlay(work: work)),
        ],
      ),
    );
    return Semantics(
      button: true,
      label: [work.heading, line].where((part) => part.isNotEmpty).join(', '),
      excludeSemantics: true,
      child: InkWell(
        onTap: () => openWork(work),
        borderRadius: BorderRadius.circular(NfRadii.m),
        child: wide
            ? frame
            : Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  frame,
                  const SizedBox(height: 8),
                  _Caption(heading: work.heading, line: line),
                ],
              ),
      ),
    );
  }
}

class _Caption extends StatelessWidget {
  const _Caption({required this.heading, required this.line});
  final String heading;
  final String line;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    mainAxisSize: MainAxisSize.min,
    children: [
      Text(
        heading,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: nfText(13.5, weight: 600, height: 1.25),
      ),
      if (line.isNotEmpty)
        Text(
          line,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: nfText(12, color: NfColors.text3, height: 1.3),
        ),
    ],
  );
}

class _WatchOverlay extends SignalWidget {
  const _WatchOverlay({required this.work});
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    final data = getIt<WatchStateService>().of(work);
    final position = data.positionMs ?? 0;
    final length = work.runtimeMs ?? 0;
    final progress = !data.played && position > 0 && length > 0
        ? (position / length).clamp(0.0, 1.0)
        : null;
    return Stack(
      fit: StackFit.expand,
      children: [
        if (data.played)
          Positioned(
            left: 8,
            top: 8,
            child: Container(
              width: 22,
              height: 22,
              decoration: const BoxDecoration(
                color: NfColors.accent,
                shape: BoxShape.circle,
              ),
              child: const Icon(
                PhosphorIconsBold.check,
                size: 13,
                color: NfColors.accentInk,
              ),
            ),
          ),
        if (progress != null)
          Positioned(
            left: 8,
            right: 8,
            bottom: 8,
            child: ClipRRect(
              borderRadius: BorderRadius.circular(2),
              child: LinearProgressIndicator(
                value: progress,
                minHeight: 3,
                color: NfColors.accent,
                backgroundColor: const Color(0x52FFFFFF),
              ),
            ),
          ),
      ],
    );
  }
}

class _QuickPlay extends SignalWidget {
  const _QuickPlay({required this.work});
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    // Hovering is how the web reveals this; touch has the detail page instead.
    if (context.windowClass.isCompact) return const SizedBox.shrink();
    final playback = getIt<PlaybackService>();
    final busy = playback.preparing.value == work.id;
    return Material(
      color: const Color(0xB3000000),
      shape: const CircleBorder(),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        onTap: busy ? null : () => playback.playWork(work),
        child: SizedBox.square(
          dimension: 32,
          child: busy
              ? const Padding(
                  padding: EdgeInsets.all(8),
                  child: CircularProgressIndicator(
                    strokeWidth: 2,
                    color: NfColors.text,
                  ),
                )
              : const Icon(
                  PhosphorIconsFill.play,
                  size: 14,
                  color: NfColors.text,
                ),
        ),
      ),
    );
  }
}
