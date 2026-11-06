// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../di.dart';
import '../features/detail/work_detail_screen.dart';
import '../features/player/playback_service.dart';
import '../layout/window.dart';
import '../models/work.dart';
import '../services/indexing_service.dart';
import '../services/nav_service.dart';
import '../services/watch_state.dart';
import '../theme.dart';
import '../utils/format.dart';
import '../utils/images.dart';
import 'nf_press.dart';
import 'poster_flight.dart';

void openWork(WorkSummary work, {PosterFlight? flight}) =>
    getIt<NavService>().push(
      WorkDetailScreen(
        workId: work.id,
        server: work.server,
        seed: work,
        flight: flight,
      ),
    );

/// A poster, or a wide thumbnail for shelves like Continue Watching. Shows
/// progress and watched state as they change on this device.
class WorkCard extends StatefulWidget {
  const WorkCard({super.key, required this.work, this.wide = false, this.meta});

  final WorkSummary work;
  final bool wide;
  final String? meta;

  @override
  State<WorkCard> createState() => _WorkCardState();
}

class _WorkCardState extends State<WorkCard> {
  final _press = Press();
  final _hovered = signal(false);
  final _posterTag = Object();

  /// Whether the page this opens draws the same picture in the same shape: an
  /// episode's wide frame, or anything else's poster.
  bool get _flies => widget.wide == widget.work.isEpisode;

  @override
  void dispose() {
    _press.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (widget.work.indexing == null) return _card(context, widget.work);
    // A match brings its title and year along with the art.
    return SignalBuilder(
      builder: (context) =>
          _card(context, getIt<IndexingService>().latest(widget.work)),
    );
  }

  Widget _card(BuildContext context, WorkSummary work) {
    final WorkCard(:wide, :meta) = widget;
    final line =
        meta ??
        [
          work.subtitle,
          work.episodeTitle,
          if (!work.isEpisode) titleCase(work.kind),
          ?work.server?.name,
        ].where((part) => part.isNotEmpty).join(' · ');
    final still = MediaQuery.disableAnimationsOf(context);
    final picture = AspectRatio(
      aspectRatio: wide ? 16 / 9 : 2 / 3,
      child: Stack(
        fit: StackFit.expand,
        children: [
          PosterHero(
            tag: _flies ? _posterTag : null,
            work: work,
            kind: wide ? ArtKind.thumb : ArtKind.poster,
            radius: NfRadii.m,
          ),
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
    final frame = NfSink(
      press: _press,
      child: SignalBuilder(
        builder: (_) {
          final hovered = _hovered.value;
          return AnimatedContainer(
            duration: still ? Duration.zero : const Duration(milliseconds: 400),
            curve: NfMotion.easeOut,
            transform: Matrix4.translationValues(0, hovered ? -4 : 0, 0),
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(NfRadii.m),
              boxShadow: [
                hovered
                    ? const BoxShadow(
                        color: Color(0xF2000000),
                        offset: Offset(0, 26),
                        blurRadius: 44,
                        spreadRadius: -18,
                      )
                    : const BoxShadow(
                        color: Color(0xE6000000),
                        offset: Offset(0, 8),
                        blurRadius: 22,
                        spreadRadius: -14,
                      ),
              ],
            ),
            child: picture,
          );
        },
      ),
    );
    return Semantics(
      button: true,
      label: [work.heading, line].where((part) => part.isNotEmpty).join(', '),
      excludeSemantics: true,
      child: InkWell(
        onTap: () => openWork(
          work,
          flight: _flies
              ? PosterFlight(tag: _posterTag, width: context.size!.width)
              : null,
        ),
        onHighlightChanged: _press.update,
        onHover: (hovering) => _hovered.value = hovering,
        // The lift says it instead, and a tint would show around the poster.
        hoverColor: Colors.transparent,
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
    return NfPressable(
      scale: 0.9,
      builder: (context, onPress) => Material(
        color: const Color(0xB3000000),
        shape: const CircleBorder(),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: busy ? null : () => playback.playWork(work),
          onHighlightChanged: onPress,
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
      ),
    );
  }
}
