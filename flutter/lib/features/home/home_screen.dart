// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../models/work.dart';
import '../../services/session.dart';
import '../../services/watch_state.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../utils/images.dart';
import '../../widgets/artwork.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_state.dart';
import '../../widgets/rail.dart';
import '../../widgets/work_card.dart';
import '../player/playback_service.dart';
import 'home_service.dart';

class HomeScreen extends SignalWidget {
  const HomeScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final home = getIt<HomeService>();
    final session = getIt<SessionService>();
    final shelves = home.shelves.value;
    if (home.query.waiting.value) return const _HomeSkeleton();
    final failure = home.query.error.value;
    if (shelves == null) {
      return NfState.error(
        title: 'Home is out of reach',
        message: failure?.toString(),
        retry: home.refresh,
      );
    }
    if (shelves.isEmpty) {
      final admin = session.viewer.value?.isAdmin ?? false;
      return RefreshIndicator(
        onRefresh: () async => home.refresh(),
        child: ListView(
          children: [
            SizedBox(
              height: MediaQuery.sizeOf(context).height * 0.7,
              child: NfState(
                icon: PhosphorIconsRegular.sparkle,
                title: 'Welcome, ${session.viewer.value?.name ?? 'there'}.',
                message: admin
                    ? 'Your libraries are empty so far. Add a library and run a scan from Administration.'
                    : 'Nothing has been shared with you yet. Ask the server’s administrator to grant you a library.',
              ),
            ),
          ],
        ),
      );
    }
    final featured = home.featured.value;
    return RefreshIndicator(
      onRefresh: () async => home.refresh(),
      child: CustomScrollView(
        slivers: [
          if (featured.isNotEmpty)
            SliverToBoxAdapter(child: _Hero(items: featured)),
          if (home.unreachable.value.isNotEmpty)
            SliverToBoxAdapter(
              child: Padding(
                padding: EdgeInsets.fromLTRB(
                  context.gutter,
                  20,
                  context.gutter,
                  0,
                ),
                child: Text(
                  '${home.unreachable.value.join(', ')} can’t be reached right now.',
                  style: NfType.small,
                ),
              ),
            ),
          if (shelves.continueWatching.isNotEmpty)
            SliverToBoxAdapter(
              child: Rail(
                title: 'Continue watching',
                items: shelves.continueWatching,
                wide: true,
              ),
            ),
          if (shelves.recentlyAdded.isNotEmpty)
            SliverToBoxAdapter(
              child: Rail(
                title: 'Recently added',
                items: shelves.recentlyAdded,
              ),
            ),
          if (shelves.recommendations.isNotEmpty)
            SliverToBoxAdapter(
              child: Rail(
                title: 'Recommended',
                eyebrow: 'Because of what you watch',
                items: shelves.recommendations,
              ),
            ),
          if (shelves.recentlyPlayed.isNotEmpty)
            SliverToBoxAdapter(
              child: Rail(title: 'Watch again', items: shelves.recentlyPlayed),
            ),
          const SliverToBoxAdapter(child: SizedBox(height: 40)),
        ],
      ),
    );
  }
}

class _HomeSkeleton extends StatelessWidget {
  const _HomeSkeleton();

  @override
  Widget build(BuildContext context) => ListView(
    physics: const NeverScrollableScrollPhysics(),
    children: [
      NfSkeleton(height: MediaQuery.sizeOf(context).height * 0.45, radius: 0),
      const RailSkeleton(),
      const RailSkeleton(),
    ],
  );
}

/// The featured carousel. Slides advance on their own until the viewer takes
/// over by swiping.
class _Hero extends StatefulWidget {
  const _Hero({required this.items});
  final List<WorkSummary> items;

  @override
  State<_Hero> createState() => _HeroState();
}

class _HeroState extends State<_Hero> {
  static const _dwell = Duration(seconds: 9);
  final _controller = PageController();
  final _index = signal(0);
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    _schedule();
  }

  void _schedule() {
    _timer?.cancel();
    if (widget.items.length < 2) return;
    _timer = Timer.periodic(_dwell, (_) {
      if (!_controller.hasClients) return;
      final next = (_index.value + 1) % widget.items.length;
      unawaited(
        _controller.animateToPage(
          next,
          duration: NfMotion.slow,
          curve: NfMotion.easeInOut,
        ),
      );
    });
  }

  @override
  void dispose() {
    _timer?.cancel();
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final size = MediaQuery.sizeOf(context);
    final height = (size.height * 0.62).clamp(380.0, 640.0);
    return SizedBox(
      height: height,
      child: Stack(
        fit: StackFit.expand,
        children: [
          PageView.builder(
            controller: _controller,
            itemCount: widget.items.length,
            onPageChanged: (index) {
              _index.value = index;
              _schedule();
            },
            itemBuilder: (context, index) =>
                _HeroSlide(work: widget.items[index]),
          ),
          if (widget.items.length > 1)
            Positioned(
              right: context.gutter,
              bottom: 22,
              child: SignalBuilder(
                builder: (_) => Row(
                  children: [
                    for (var i = 0; i < widget.items.length; i++)
                      AnimatedContainer(
                        duration: NfMotion.base,
                        margin: const EdgeInsets.only(left: 6),
                        width: i == _index.value ? 18 : 6,
                        height: 6,
                        decoration: BoxDecoration(
                          color: i == _index.value
                              ? NfColors.accent
                              : const Color(0x52FFFFFF),
                          borderRadius: BorderRadius.circular(3),
                        ),
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

class _HeroSlide extends StatelessWidget {
  const _HeroSlide({required this.work});
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    final hasLogo = pickImage(work.images, ArtKind.logo) != null;
    final meta = [
      work.year?.toString(),
      titleCase(work.kind),
      runtime(work.runtimeMs),
    ].whereType<String>().where((part) => part.isNotEmpty).join(' · ');
    return Stack(
      fit: StackFit.expand,
      children: [
        Artwork(work: work, kind: ArtKind.backdrop, radius: 0),
        const DecoratedBox(
          decoration: BoxDecoration(
            gradient: LinearGradient(
              begin: Alignment.topCenter,
              end: Alignment.bottomCenter,
              stops: [0.2, 0.65, 1],
              colors: [Color(0x3307080B), Color(0x9907080B), NfColors.bg],
            ),
          ),
        ),
        Positioned(
          left: context.gutter,
          right: context.gutter,
          bottom: 40,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text('JUST ADDED', style: NfType.eyebrow),
              const SizedBox(height: 8),
              if (hasLogo)
                SizedBox(
                  height: 84,
                  width: 300,
                  child: Artwork(
                    work: work,
                    kind: ArtKind.logo,
                    fit: BoxFit.contain,
                    radius: 0,
                  ),
                )
              else
                Text(
                  work.title,
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  style: NfType.display,
                ),
              const SizedBox(height: 8),
              Row(
                children: [
                  if (work.communityRating != null) ...[
                    const Icon(
                      PhosphorIconsFill.star,
                      size: 13,
                      color: NfColors.warning,
                    ),
                    const SizedBox(width: 4),
                    Text(
                      work.communityRating!.toStringAsFixed(1),
                      style: nfText(13.5, weight: 600, color: NfColors.warning),
                    ),
                    const SizedBox(width: 10),
                  ],
                  Expanded(
                    child: Text(
                      meta,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: nfText(13.5, weight: 550, color: NfColors.text2),
                    ),
                  ),
                ],
              ),
              if (work.overview != null) ...[
                const SizedBox(height: 8),
                ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 560),
                  child: Text(
                    work.overview!,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: NfType.bodyMuted,
                  ),
                ),
              ],
              const SizedBox(height: 14),
              Row(
                children: [
                  _HeroPlay(work: work),
                  const SizedBox(width: 10),
                  NfButton(
                    label: 'More info',
                    icon: const Icon(PhosphorIconsRegular.info),
                    style: NfButtonStyle.glass,
                    size: NfButtonSize.large,
                    onPressed: () => openWork(work),
                  ),
                ],
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _HeroPlay extends SignalWidget {
  const _HeroPlay({required this.work});
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    final playback = getIt<PlaybackService>();
    final data = getIt<WatchStateService>().of(work);
    final position = data.positionMs ?? 0;
    final resume = !data.played && position > 60000;
    return NfButton(
      label: resume ? 'Resume' : 'Play',
      icon: const Icon(PhosphorIconsFill.play),
      style: NfButtonStyle.light,
      size: NfButtonSize.large,
      busy: playback.preparing.value == work.id,
      onPressed: () => playback.playWork(work),
    );
  }
}
