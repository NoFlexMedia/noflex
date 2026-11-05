// SPDX-License-Identifier: AGPL-3.0-only
//
// A title's page. One column on a phone; two panes once there's room, split on a
// foldable's hinge so neither runs across it.

import 'package:built_collection/built_collection.dart';
import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/add_to_playlist.req.gql.dart';
import '../../graphql/queries/__generated__/collection_items.req.gql.dart';
import '../../graphql/queries/__generated__/collection_menu.req.gql.dart';
import '../../graphql/queries/__generated__/create_watch_room.req.gql.dart';
import '../../graphql/queries/__generated__/set_collection_items.req.gql.dart';
import '../../graphql/queries/__generated__/playlist_menu.req.gql.dart';
import '../../layout/window.dart';
import '../../models/media_probe.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/watch_state.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../utils/images.dart';
import '../../widgets/artwork.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../downloads/download_models.dart';
import '../downloads/downloads_service.dart';
import '../player/decoder_probe.dart';
import '../player/decoders.dart';
import '../player/playback_service.dart';
import '../player/player_screen.dart';
import '../rooms/watch_rooms_screen.dart';
import 'manage_sheet.dart';
import 'work_detail_controller.dart';

class WorkDetailScreen extends StatefulWidget {
  const WorkDetailScreen({
    super.key,
    required this.workId,
    this.server,
    this.seed,
  });

  final String workId;
  final ServerRef? server;
  final WorkSummary? seed;

  @override
  State<WorkDetailScreen> createState() => _WorkDetailScreenState();
}

class _WorkDetailScreenState extends State<WorkDetailScreen> {
  late final _controller = WorkDetailController(
    workId: widget.workId,
    server: widget.server,
    seed: widget.seed,
  );

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    extendBodyBehindAppBar: true,
    appBar: AppBar(
      backgroundColor: Colors.transparent,
      surfaceTintColor: Colors.transparent,
      foregroundColor: NfColors.text,
      actions: [_MoreMenu(controller: _controller)],
    ),
    body: SignalBuilder(
      builder: (context) {
        final work = _controller.work.value;
        final failure = _controller.query.error.value;
        if (work == null) {
          if (_controller.query.waiting.value) {
            return const _DetailSkeleton();
          }
          return failure != null
              ? NfState.error(
                  title: 'Couldn’t open this title',
                  message: failure.toString(),
                  retry: _controller.query.refetch,
                )
              : const NfState(
                  icon: PhosphorIconsRegular.magnifyingGlass,
                  title: 'This title isn’t here',
                  message: 'It may have been removed, or you don’t have access to its library.',
                );
        }
        return _Layout(controller: _controller, work: work);
      },
    ),
  );
}

/// The poster and text of a page opened with nothing to show yet. A card opens
/// it with what it already knows, so this is for everywhere else.
class _DetailSkeleton extends StatelessWidget {
  const _DetailSkeleton();

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final wide = constraints.maxWidth >= 760;
      final gutter = context.gutter;
      final poster = SizedBox(
        width: wide ? (constraints.maxWidth * 0.2).clamp(180.0, 290.0) : 150,
        child: const AspectRatio(
          aspectRatio: 2 / 3,
          child: NfSkeleton(radius: NfRadii.l),
        ),
      );
      final text = Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const NfSkeleton(width: 120, height: 18, radius: NfRadii.xs),
          const SizedBox(height: 14),
          FractionallySizedBox(
            widthFactor: 0.8,
            child: const NfSkeleton(height: 52, radius: NfRadii.s),
          ),
          const SizedBox(height: 14),
          const NfSkeleton(height: 88, radius: NfRadii.s),
        ],
      );
      return SingleChildScrollView(
        physics: const NeverScrollableScrollPhysics(),
        padding: EdgeInsets.fromLTRB(
          gutter,
          MediaQuery.paddingOf(context).top + (wide ? 72 : 64),
          gutter,
          40,
        ),
        child: wide
            ? Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  poster,
                  SizedBox(width: (constraints.maxWidth * 0.04).clamp(24, 56)),
                  Expanded(
                    child: ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 640),
                      child: text,
                    ),
                  ),
                ],
              )
            : Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [poster, const SizedBox(height: 18), text],
              ),
      );
    },
  );
}

class _Layout extends StatelessWidget {
  const _Layout({required this.controller, required this.work});

  final WorkDetailController controller;
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        if (constraints.maxWidth < 760) {
          return ListView(
            padding: EdgeInsets.zero,
            children: [
              _Summary(controller: controller, work: work),
              _Body(controller: controller, work: work),
              const SizedBox(height: 40),
            ],
          );
        }
        // A poster beside the text would put the hinge through the text, so an
        // open book keeps one pane to each side of it.
        if (context.fold?.isVertical ?? false) {
          final pad = MediaQuery.paddingOf(context).top + 56;
          return AdaptivePanes(
            primaryFraction: 0.42,
            primary: ListView(
              padding: const EdgeInsets.only(bottom: 32),
              children: [_Summary(controller: controller, work: work)],
            ),
            secondary: ListView(
              padding: EdgeInsets.only(top: pad, bottom: 40),
              children: [_Body(controller: controller, work: work)],
            ),
          );
        }
        return _Wide(
          controller: controller,
          work: work,
          width: constraints.maxWidth,
          height: constraints.maxHeight,
        );
      },
    );
  }
}

/// The poster beside the text under a banner across the whole window, with the
/// rest of the title below, as on the web.
class _Wide extends StatelessWidget {
  const _Wide({
    required this.controller,
    required this.work,
    required this.width,
    required this.height,
  });

  final WorkDetailController controller;
  final WorkSummary work;
  final double width;
  final double height;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    final posterWidth = (width * 0.2).clamp(180.0, 290.0);
    final gap = (width * 0.04).clamp(24.0, 56.0);
    final bannerHeight = (height * 0.78).clamp(0.0, 820.0);
    return SingleChildScrollView(
      child: Stack(
        // A short page mustn't cut off the banner's fade.
        clipBehavior: Clip.none,
        children: [
          Positioned(
            top: 0,
            left: 0,
            right: 0,
            height: bannerHeight,
            child: _Banner(work: work),
          ),
          Padding(
            padding: EdgeInsets.fromLTRB(
              gutter,
              MediaQuery.paddingOf(context).top + 72,
              gutter,
              40,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                _Crumbs(controller: controller),
                Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    SizedBox(
                      width: posterWidth,
                      child: _Poster(work: work),
                    ),
                    SizedBox(width: gap),
                    Expanded(
                      child: Align(
                        alignment: Alignment.topLeft,
                        child: ConstrainedBox(
                          constraints: const BoxConstraints(maxWidth: 860),
                          child: _Info(
                            controller: controller,
                            work: work,
                            withVersions: true,
                          ),
                        ),
                      ),
                    ),
                  ],
                ),
                _Body(
                  controller: controller,
                  work: work,
                  inset: false,
                  withVersions: false,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _Banner extends StatelessWidget {
  const _Banner({required this.work});
  final WorkSummary work;

  @override
  Widget build(BuildContext context) => ShaderMask(
    shaderCallback: (rect) => const LinearGradient(
      begin: Alignment.topCenter,
      end: Alignment.bottomCenter,
      stops: [0, 0.3, 1],
      colors: [Colors.white, Colors.white, Colors.transparent],
    ).createShader(rect),
    blendMode: BlendMode.dstIn,
    child: Stack(
      fit: StackFit.expand,
      children: [
        Opacity(
          opacity: 0.55,
          child: Artwork(work: work, kind: ArtKind.backdrop, radius: 0),
        ),
        DecoratedBox(
          decoration: BoxDecoration(
            gradient: LinearGradient(
              stops: const [0, 0.45, 1],
              colors: [
                NfColors.bg.withValues(alpha: 0.92),
                NfColors.bg.withValues(alpha: 0.5),
                NfColors.bg.withValues(alpha: 0.2),
              ],
            ),
          ),
        ),
      ],
    ),
  );
}

/// An episode's picture is a wide frame from it; anything else has a poster.
class _Poster extends StatelessWidget {
  const _Poster({required this.work});
  final WorkSummary work;

  @override
  Widget build(BuildContext context) => AspectRatio(
    aspectRatio: work.isEpisode ? 16 / 9 : 2 / 3,
    child: Artwork(
      work: work,
      kind: work.isEpisode ? ArtKind.thumb : ArtKind.poster,
      radius: NfRadii.l,
    ),
  );
}

class _Summary extends StatelessWidget {
  const _Summary({required this.controller, required this.work});

  final WorkDetailController controller;
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    final topInset = MediaQuery.paddingOf(context).top;
    return Stack(
      children: [
        SizedBox(
          height: 300 + topInset,
          width: double.infinity,
          child: ShaderMask(
            shaderCallback: (rect) => const LinearGradient(
              begin: Alignment.topCenter,
              end: Alignment.bottomCenter,
              colors: [Color(0x99FFFFFF), Colors.transparent],
            ).createShader(rect),
            blendMode: BlendMode.dstIn,
            child: Artwork(work: work, kind: ArtKind.backdrop, radius: 0),
          ),
        ),
        Padding(
          padding: EdgeInsets.fromLTRB(gutter, topInset + 64, gutter, 0),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                width: work.isEpisode ? 240 : 150,
                child: _Poster(work: work),
              ),
              const SizedBox(height: 18),
              _Crumbs(controller: controller),
              _Info(controller: controller, work: work),
            ],
          ),
        ),
      ],
    );
  }
}

class _Info extends StatelessWidget {
  const _Info({
    required this.controller,
    required this.work,
    this.withVersions = false,
  });

  final WorkDetailController controller;
  final WorkSummary work;
  final bool withVersions;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      _Eyebrow(controller: controller, work: work),
      const SizedBox(height: 6),
      if (pickImage(work.images, ArtKind.logo) != null)
        SizedBox(
          height: 76,
          width: 300,
          child: Artwork(
            work: work,
            kind: ArtKind.logo,
            fit: BoxFit.contain,
            radius: 0,
          ),
        )
      else
        Text(work.title, style: NfType.display),
      const SizedBox(height: 10),
      _Meta(controller: controller, work: work),
      _Overview(controller: controller),
      const SizedBox(height: 16),
      _Actions(controller: controller, work: work),
      const SizedBox(height: 16),
      _Tracks(controller: controller),
      if (withVersions) _Versions(controller: controller),
    ],
  );
}

class _Eyebrow extends SignalWidget {
  const _Eyebrow({required this.controller, required this.work});

  final WorkDetailController controller;
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    final unconfirmed =
        controller.query.data.value?.work?.metadataStatus ==
        GMetadataStatus.UNCONFIRMED;
    return Text.rich(
      TextSpan(
        text: titleCase(work.kind).toUpperCase(),
        children: [
          if (unconfirmed)
            TextSpan(
              text: ' · UNCONFIRMED MATCH',
              style: NfType.eyebrow.copyWith(color: NfColors.warning),
            ),
        ],
      ),
      style: NfType.eyebrow,
    );
  }
}

class _Crumbs extends SignalWidget {
  const _Crumbs({required this.controller});
  final WorkDetailController controller;

  @override
  Widget build(BuildContext context) {
    final crumbs = controller.crumbs.value;
    if (crumbs.isEmpty) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Wrap(
        crossAxisAlignment: WrapCrossAlignment.center,
        spacing: 4,
        children: [
          for (final crumb in crumbs) ...[
            InkWell(
              onTap: () => getIt<NavService>().push(
                WorkDetailScreen(
                  workId: crumb.id,
                  server: controller.server,
                  seed: crumb,
                ),
              ),
              child: Text(
                crumb.title,
                style: nfText(13.5, weight: 550, color: NfColors.text2),
              ),
            ),
            const Icon(
              PhosphorIconsRegular.caretRight,
              size: 13,
              color: NfColors.text3,
            ),
          ],
        ],
      ),
    );
  }
}

class _Meta extends SignalWidget {
  const _Meta({required this.controller, required this.work});

  final WorkDetailController controller;
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    final detail = controller.query.data.value?.work;
    final runtimeMs = controller.edition.value?.runtimeMs ?? work.runtimeMs;
    final tags = detail?.tags.toList() ?? const [];
    final parental = tags
        .where((tag) => tag.namespace == 'parental')
        .firstOrNull
        ?.display;
    final genres = tags
        .where((tag) => tag.namespace == 'genre')
        .map((tag) => tag.display)
        .toList();
    final version = controller.version.value;
    final parts = [
      if (work.subtitle != work.year?.toString()) work.subtitle,
      work.year?.toString() ?? '',
      runtime(runtimeMs),
    ].where((part) => part.isNotEmpty).join(' · ');
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Wrap(
          spacing: 12,
          runSpacing: 6,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            if (work.communityRating != null)
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const Icon(
                    PhosphorIconsFill.star,
                    size: 14,
                    color: NfColors.warning,
                  ),
                  const SizedBox(width: 4),
                  Text(
                    work.communityRating!.toStringAsFixed(1),
                    style: nfText(14, weight: 600, color: NfColors.warning),
                  ),
                ],
              ),
            if (parts.isNotEmpty)
              Text(
                parts,
                style: nfText(14, weight: 550, color: NfColors.text2),
              ),
            if (parental != null) NfBadge(parental),
          ],
        ),
        if (genres.isNotEmpty) ...[
          const SizedBox(height: 6),
          Text(genres.join(' · '), style: nfText(13.5, color: NfColors.text3)),
        ],
        if (!work.isContainer && version != null) ...[
          const SizedBox(height: 10),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              for (final badge in version.probe.techBadges(
                controller.chosenAudio.value,
              ))
                NfBadge(badge),
            ],
          ),
        ],
      ],
    );
  }
}

class _Overview extends SignalWidget {
  const _Overview({required this.controller});
  final WorkDetailController controller;

  @override
  Widget build(BuildContext context) {
    final detail = controller.query.data.value?.work;
    final overview = detail?.overview;
    final tagline = detail?.tagline;
    final expanded = controller.overviewExpanded.value;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (tagline != null && tagline.isNotEmpty) ...[
          const SizedBox(height: 12),
          Text(
            tagline,
            style: nfText(
              15.5,
              color: NfColors.text2,
              fontStyle: FontStyle.italic,
            ),
          ),
        ],
        if (overview != null && overview.isNotEmpty) ...[
          const SizedBox(height: 10),
          InkWell(
            onTap: () => controller.overviewExpanded.value = !expanded,
            child: AnimatedSize(
              duration: NfMotion.base,
              alignment: Alignment.topCenter,
              child: Text(
                overview,
                maxLines: expanded ? null : 4,
                overflow: expanded
                    ? TextOverflow.visible
                    : TextOverflow.ellipsis,
                style: NfType.bodyMuted,
              ),
            ),
          ),
        ],
      ],
    );
  }
}

class _Actions extends SignalWidget {
  const _Actions({required this.controller, required this.work});

  final WorkDetailController controller;
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    final playback = getIt<PlaybackService>();
    final watch = getIt<WatchStateService>().of(work);
    final up = controller.upNext.value;
    final isContainer = work.isContainer;
    final resumeMs = !watch.played ? watch.positionMs ?? 0 : 0;
    final length = controller.edition.value?.runtimeMs ?? work.runtimeMs ?? 0;
    final progress = length > 0 && resumeMs > 0
        ? (resumeMs / length).clamp(0.0, 1.0)
        : 0.0;
    return Wrap(
      spacing: 10,
      runSpacing: 10,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        if (up != null)
          NfButton(
            label:
                '${up.rewatch
                    ? 'Watch again'
                    : up.resume
                    ? 'Resume'
                    : 'Play'}'
                '${up.work.subtitle.isNotEmpty ? ' ${up.work.subtitle}' : ''}',
            icon: const Icon(PhosphorIconsFill.play),
            style: NfButtonStyle.light,
            size: NfButtonSize.large,
            busy: playback.preparing.value == up.work.id,
            onPressed: () => playback.playWork(up.work),
          ),
        if (!isContainer)
          Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              NfButton(
                label: resumeMs > 30000
                    ? 'Resume from ${timecode(resumeMs)}'
                    : 'Play',
                icon: const Icon(PhosphorIconsFill.play),
                style: NfButtonStyle.light,
                size: NfButtonSize.large,
                busy: playback.preparing.value == work.id,
                onPressed: controller.version.value == null && !_hasOffline
                    ? null
                    : () => controller.play(),
              ),
              if (progress > 0)
                Padding(
                  padding: const EdgeInsets.only(top: 6),
                  child: SizedBox(
                    width: 130,
                    child: NfProgressBar(value: progress, height: 3),
                  ),
                ),
            ],
          ),
        if (!isContainer && resumeMs > 30000)
          NfIconButton.glass(
            icon: PhosphorIconsRegular.clockCounterClockwise,
            label: 'Play from the beginning',
            onPressed: () => controller.play(fromStart: true),
          ),
        NfIconButton.glass(
          icon: watch.favorite
              ? PhosphorIconsFill.heart
              : PhosphorIconsRegular.heart,
          label: watch.favorite ? 'Remove from favorites' : 'Add to favorites',
          color: watch.favorite ? NfColors.heart : NfColors.text,
          onPressed: controller.toggleFavorite,
        ),
        NfIconButton.glass(
          icon: watch.played
              ? PhosphorIconsFill.checkCircle
              : PhosphorIconsRegular.check,
          label: watch.played ? 'Mark unwatched' : 'Mark watched',
          color: watch.played ? NfColors.accent : NfColors.text,
          onPressed: controller.togglePlayed,
        ),
        if (!isContainer && (controller.downloadsAllowed.value || _isOnDevice))
          _DownloadButton(controller: controller, work: work),
      ],
    );
  }

  bool get _hasOffline =>
      getIt<DownloadsService>().completedFor(work.id) != null;

  bool get _isOnDevice =>
      getIt<DownloadsService>().itemsFor(work.id).isNotEmpty;
}

class _DownloadButton extends SignalWidget {
  const _DownloadButton({required this.controller, required this.work});

  final WorkDetailController controller;
  final WorkSummary work;

  Future<void> _start() async {
    final version = controller.version.value;
    if (version == null) return;
    final downloads = getIt<DownloadsService>();
    final started = await getIt<ToastService>().attempt(
      () => downloads.download(version, seed: work, server: controller.server),
    );
    if (started == false) {
      final edition = controller.edition.value;
      if (edition == null) return;
      final confirmed = await confirmAction(
        title: 'Nothing’s ready for this device yet',
        message: 'The server can package this version as one file to save here. It keeps the original’s format, so this device may not play it well. It’ll be under You → Offline copies when it’s ready.',
        confirmLabel: 'Export',
      );
      if (confirmed) {
        await downloads.createExport(edition.id, server: controller.server);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final items = getIt<DownloadsService>().itemsFor(work.id);
    final item = items.isEmpty ? null : items.first;
    if (item == null) {
      return NfIconButton.glass(
        icon: PhosphorIconsRegular.downloadSimple,
        label: 'Download',
        onPressed: controller.version.value == null ? null : _start,
      );
    }
    if (item.isFinished) {
      return NfIconButton.glass(
        icon: PhosphorIconsFill.checkCircle,
        color: NfColors.success,
        label: 'Downloaded. Tap to remove.',
        onPressed: () async {
          final confirmed = await confirmAction(
            title: 'Remove this download?',
            message: 'It’ll need downloading again to watch offline.',
            confirmLabel: 'Remove',
            danger: true,
          );
          if (confirmed) await getIt<DownloadsService>().remove(item);
        },
      );
    }
    return SizedBox.square(
      dimension: 50,
      child: Stack(
        alignment: Alignment.center,
        children: [
          CircularProgressIndicator(
            value: item.status == OfflineStatus.downloading
                ? item.progress
                : null,
            strokeWidth: 2.5,
            color: NfColors.accent,
            backgroundColor: NfColors.surface3,
          ),
          const Icon(
            PhosphorIconsRegular.downloadSimple,
            size: 18,
            color: NfColors.text2,
          ),
        ],
      ),
    );
  }
}

class _Tracks extends SignalWidget {
  const _Tracks({required this.controller});
  final WorkDetailController controller;

  @override
  Widget build(BuildContext context) {
    if (controller.isContainer.value) return const SizedBox.shrink();
    final audio = controller.audioStreams.value;
    final subtitles = controller.subtitleTracks.value;
    if (audio.length < 2 && subtitles.isEmpty) return const SizedBox.shrink();
    final chosenSubtitle = controller.subtitleChoice.value;
    return Wrap(
      spacing: 8,
      runSpacing: 8,
      children: [
        if (audio.length > 1)
          NfChoiceChip(
            label: _audioLabel(controller.chosenAudio.value),
            selected: false,
            leading: const Icon(PhosphorIconsRegular.speakerHigh),
            onTap: () async {
              final choice = await pickOption<int>(
                title: 'Audio',
                selected: controller.audioChoice.value,
                options: [
                  for (final stream in audio)
                    NfOption(
                      stream.index,
                      _audioLabel(stream),
                      detail: stream.format,
                    ),
                ],
              );
              if (choice != null) controller.audioChoice.value = choice;
            },
          ),
        if (subtitles.isNotEmpty)
          NfChoiceChip(
            label: chosenSubtitle == null
                ? 'No subtitles'
                : subtitles
                          .where((t) => t.index == chosenSubtitle)
                          .firstOrNull
                          ?.label ??
                      'Subtitles',
            selected: false,
            leading: const Icon(PhosphorIconsRegular.closedCaptioning),
            onTap: () async {
              const off = -1;
              final choice = await pickOption<int>(
                title: 'Subtitles',
                selected: chosenSubtitle ?? off,
                options: [
                  const NfOption(off, 'Off'),
                  for (final track in subtitles)
                    NfOption(
                      track.index,
                      track.label,
                      detail: track.forced ? 'Forced' : null,
                    ),
                ],
              );
              if (choice != null) {
                controller.subtitleChoice.value = choice == off ? null : choice;
              }
            },
          ),
      ],
    );
  }

  static String _audioLabel(AudioProbe? stream) {
    if (stream == null) return 'Audio';
    final name = languageName(stream.language);
    return name.isNotEmpty
        ? name
        : (stream.title ?? 'Track ${stream.index + 1}');
  }
}

class _Body extends StatelessWidget {
  const _Body({
    required this.controller,
    required this.work,
    this.inset = true,
    this.withVersions = true,
  });

  final WorkDetailController controller;
  final WorkSummary work;

  /// Whether this brings its own gutters, rather than sitting inside a page's.
  final bool inset;
  final bool withVersions;

  @override
  Widget build(BuildContext context) => Padding(
    padding: EdgeInsets.fromLTRB(
      inset ? context.gutter : 0,
      12,
      inset ? context.gutter : 0,
      0,
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (withVersions) _Versions(controller: controller),
        if (work.isContainer) _Children(controller: controller),
        _Credits(controller: controller),
        if (!work.isContainer) _TechSpecs(controller: controller),
      ],
    ),
  );
}

/// Editions and versions, each marked with how this device would decode it.
class _Versions extends SignalWidget {
  const _Versions({required this.controller});
  final WorkDetailController controller;

  @override
  Widget build(BuildContext context) {
    if (controller.isContainer.value) return const SizedBox.shrink();
    final editions = controller.editions.value;
    final versions = controller.versions.value;
    final multiPart = controller.multiPart.value;
    final profile = getIt<DecoderProbeService>().profile.value;
    if (editions.length < 2 && versions.length < 2) {
      final failure = controller.query.error.value;
      if (editions.isEmpty &&
          !controller.query.loading.value &&
          failure == null) {
        return Padding(
          padding: const EdgeInsets.only(top: 14),
          child: Text(
            'No playable files are attached to this title yet.',
            style: NfType.small,
          ),
        );
      }
      return const SizedBox.shrink();
    }
    return Padding(
      padding: const EdgeInsets.only(top: 14),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (editions.length > 1) ...[
            Text('EDITION', style: NfType.eyebrow),
            const SizedBox(height: 8),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                for (final edition in editions)
                  NfChoiceChip(
                    label: edition.name,
                    detail: runtime(edition.runtimeMs).isEmpty
                        ? null
                        : runtime(edition.runtimeMs),
                    selected: controller.edition.value?.id == edition.id,
                    onTap: () => controller.chooseEdition(edition.id),
                  ),
              ],
            ),
            const SizedBox(height: 16),
          ],
          if (versions.length > 1) ...[
            Text(multiPart ? 'PART' : 'VERSION', style: NfType.eyebrow),
            const SizedBox(height: 8),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                for (final version in versions)
                  NfChoiceChip(
                    label: [
                      if (multiPart) 'Part ${version.partIndex + 1}',
                      version.label ?? version.fileName,
                    ].join(' · '),
                    detail: bytes(version.sizeBytes),
                    selected: controller.version.value?.id == version.id,
                    leading: _tierIcon(version, profile),
                    onTap: () => controller.chooseVersion(version.id),
                  ),
              ],
            ),
          ],
        ],
      ),
    );
  }

  static Widget? _tierIcon(MediaVersion version, DecoderProfile? profile) {
    final video = version.probe.video;
    if (video == null || profile == null) return null;
    return profile.tierOf(video) == DecodeTier.hardware
        ? const Tooltip(
            message: 'Plays in hardware on this device',
            child: Icon(PhosphorIconsFill.lightning),
          )
        : null;
  }
}

class _Children extends SignalWidget {
  const _Children({required this.controller});
  final WorkDetailController controller;

  @override
  Widget build(BuildContext context) {
    final seasons = controller.seasons.value;
    final episodes = controller.episodes.value;
    final selected = controller.selectedSeason.value;
    return Padding(
      padding: const EdgeInsets.only(top: 18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (seasons.length > 1) ...[
            SizedBox(
              height: 38,
              child: ListView(
                scrollDirection: Axis.horizontal,
                children: [
                  for (final season in seasons)
                    Padding(
                      padding: const EdgeInsets.only(right: 8),
                      child: NfChoiceChip(
                        label: season.title,
                        selected: selected?.id == season.id,
                        onTap: () => controller.chooseSeason(season),
                      ),
                    ),
                ],
              ),
            ),
            const SizedBox(height: 12),
          ] else if (selected != null) ...[
            NfSectionHead(selected.title),
            const SizedBox(height: 10),
          ],
          if (controller.loadingEpisodes.value)
            const NfSkeletonRows(height: 84)
          else
            for (final episode in episodes) _EpisodeRow(work: episode),
        ],
      ),
    );
  }
}

class _EpisodeRow extends SignalWidget {
  const _EpisodeRow({required this.work});
  final WorkSummary work;

  @override
  Widget build(BuildContext context) {
    final playback = getIt<PlaybackService>();
    final data = getIt<WatchStateService>().of(work);
    final downloads = getIt<DownloadsService>().itemsFor(work.id);
    final position = data.positionMs ?? 0;
    final length = work.runtimeMs ?? 0;
    final progress = !data.played && position > 0 && length > 0
        ? (position / length).clamp(0.0, 1.0)
        : null;
    final isPlayable = !work.isContainer;
    final windowWidth = MediaQuery.sizeOf(context).width;
    final thumbWidth = windowWidth < 760
        ? 132.0
        : (windowWidth * 0.18).clamp(150.0, 240.0);
    return InkWell(
      borderRadius: BorderRadius.circular(NfRadii.m),
      onTap: () => isPlayable
          ? playback.playWork(work)
          : getIt<NavService>().push(
              WorkDetailScreen(
                workId: work.id,
                server: work.server,
                seed: work,
              ),
            ),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 8),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: thumbWidth,
              child: AspectRatio(
                aspectRatio: 16 / 9,
                child: Stack(
                  fit: StackFit.expand,
                  children: [
                    Artwork(work: work, kind: ArtKind.thumb, radius: NfRadii.s),
                    if (data.played)
                      const Positioned(
                        left: 6,
                        top: 6,
                        child: Icon(
                          PhosphorIconsFill.checkCircle,
                          size: 18,
                          color: NfColors.accent,
                        ),
                      ),
                    if (progress != null)
                      Positioned(
                        left: 6,
                        right: 6,
                        bottom: 6,
                        child: NfProgressBar(value: progress, height: 3),
                      ),
                    if (playback.preparing.value == work.id)
                      const Center(
                        child: SizedBox.square(
                          dimension: 22,
                          child: CircularProgressIndicator(
                            strokeWidth: 2.5,
                            color: NfColors.text,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    work.indexNumber != null
                        ? '${work.indexNumber}. ${work.title}'
                        : work.title,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: nfText(14.5, weight: 600),
                  ),
                  if (runtime(work.runtimeMs).isNotEmpty) ...[
                    const SizedBox(height: 2),
                    Text(runtime(work.runtimeMs), style: NfType.caption),
                  ],
                  if (work.overview != null && work.overview!.isNotEmpty) ...[
                    const SizedBox(height: 4),
                    Text(
                      work.overview!,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: NfType.small,
                    ),
                  ],
                ],
              ),
            ),
            if (downloads.any((item) => item.isFinished))
              const Padding(
                padding: EdgeInsets.only(left: 8, top: 2),
                child: Icon(
                  PhosphorIconsFill.checkCircle,
                  size: 16,
                  color: NfColors.success,
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _Credits extends SignalWidget {
  const _Credits({required this.controller});
  final WorkDetailController controller;

  @override
  Widget build(BuildContext context) {
    final credits =
        controller.query.data.value?.work?.credits.toList() ?? const [];
    if (credits.isEmpty) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(top: 26),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const NfSectionHead('Cast & crew'),
          const SizedBox(height: 10),
          SizedBox(
            height: 58,
            child: ListView.separated(
              scrollDirection: Axis.horizontal,
              itemCount: credits.length,
              separatorBuilder: (_, _) => const SizedBox(width: 18),
              itemBuilder: (_, index) {
                final credit = credits[index];
                return ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 170),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        credit.name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: nfText(14, weight: 600),
                      ),
                      Text(
                        credit.character ?? titleCase(credit.role ?? ''),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: NfType.caption,
                      ),
                    ],
                  ),
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}

class _TechSpecs extends SignalWidget {
  const _TechSpecs({required this.controller});
  final WorkDetailController controller;

  @override
  Widget build(BuildContext context) {
    final edition = controller.edition.value;
    final version = controller.version.value;
    final video = version?.probe.video;
    final profile = getIt<DecoderProbeService>().profile.value;
    final cues = edition?.trickplay?.chapterCues.toList() ?? const [];
    final chapters = edition?.chapters.toList() ?? const [];
    final facts = <(String, String)>[
      if (video != null) ...[
        (
          'Video',
          [
            codecName(video.codec),
            profileName(video.profile),
            levelName(video.codec, video.level),
          ].whereType<String>().where((s) => s.isNotEmpty).join(' · '),
        ),
        (
          'Resolution',
          '${video.width} × ${video.height}${video.frameRate != null ? ' @ ${video.frameRate!.toStringAsFixed(3).replaceFirst(RegExp(r'\.?0+$'), '')} fps' : ''}',
        ),
        (
          'Dynamic range',
          video.dynamicRange.isEmpty ? 'SDR' : video.dynamicRange,
        ),
        if (video.bitDepth != null) ('Bit depth', '${video.bitDepth}-bit'),
        if (profile != null)
          (
            'On this device',
            switch (profile.tierOf(video)) {
              DecodeTier.hardware => 'Plays in hardware',
              DecodeTier.software => 'Plays in software',
              DecodeTier.server => 'The server will convert it',
            },
          ),
      ],
      if (controller.chosenAudio.value != null)
        ('Audio', controller.chosenAudio.value!.format),
      if (version?.container != null)
        ('Container', version!.container!.toUpperCase()),
      if (version != null) ('File size', bytes(version.sizeBytes)),
    ];
    if (facts.isEmpty && cues.isEmpty && chapters.isEmpty) {
      return const SizedBox.shrink();
    }
    return Padding(
      padding: const EdgeInsets.only(top: 26),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (cues.isNotEmpty || chapters.isNotEmpty) ...[
            const NfSectionHead('Chapters'),
            const SizedBox(height: 10),
            SizedBox(
              height: cues.isNotEmpty ? 150 : 56,
              child: ListView.separated(
                scrollDirection: Axis.horizontal,
                itemCount: cues.isNotEmpty ? cues.length : chapters.length,
                separatorBuilder: (_, _) => const SizedBox(width: 12),
                itemBuilder: (_, index) {
                  if (cues.isNotEmpty) {
                    final cue = cues[index];
                    return InkWell(
                      onTap: () => controller.play(positionMs: cue.positionMs),
                      child: SizedBox(
                        width: 176,
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            SpriteThumb(
                              url: cue.sheetUrl,
                              server: controller.server?.origin,
                              x: cue.x,
                              y: cue.y,
                              width: cue.width,
                              height: cue.height,
                              displayWidth: 176,
                            ),
                            const SizedBox(height: 6),
                            Text(
                              cue.title ?? 'Chapter ${index + 1}',
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: nfText(13, weight: 600),
                            ),
                            Text(
                              timecode(cue.positionMs),
                              style: NfType.caption,
                            ),
                          ],
                        ),
                      ),
                    );
                  }
                  final chapter = chapters[index];
                  return NfChoiceChip(
                    label: chapter.title ?? 'Chapter ${index + 1}',
                    detail: timecode(chapter.positionMs),
                    selected: false,
                    onTap: () =>
                        controller.play(positionMs: chapter.positionMs),
                  );
                },
              ),
            ),
            const SizedBox(height: 24),
          ],
          const NfSectionHead('Technical'),
          const SizedBox(height: 10),
          Wrap(
            spacing: 28,
            runSpacing: 14,
            children: [
              for (final (term, value) in facts)
                SizedBox(
                  width: 170,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(term.toUpperCase(), style: NfType.eyebrow),
                      const SizedBox(height: 3),
                      Text(value, style: nfText(14, weight: 550)),
                    ],
                  ),
                ),
            ],
          ),
        ],
      ),
    );
  }
}

class _MoreMenu extends SignalWidget {
  const _MoreMenu({required this.controller});
  final WorkDetailController controller;

  @override
  Widget build(BuildContext context) {
    final work = controller.work.value;
    if (work == null) return const SizedBox.shrink();
    return IconButton(
      tooltip: 'More actions',
      icon: const Icon(PhosphorIconsBold.dotsThree),
      onPressed: () async {
        const playlist = 'playlist';
        const collection = 'collection';
        const room = 'room';
        const exportMp4 = 'mp4';
        const exportMkv = 'mkv';
        const manage = 'manage';
        // A linked server's title belongs to that server's administrator, and
        // its rooms to that server's list.
        final linked = controller.server != null;
        final manageable =
            !linked && (getIt<SessionService>().viewer.value?.isAdmin ?? false);
        final exportable =
            !work.isContainer && controller.downloadsAllowed.value;
        final choice = await pickOption<String>(
          title: work.title,
          options: [
            const NfOption(
              playlist,
              'Add to playlist',
              leading: Icon(PhosphorIconsRegular.cardsThree, size: 18),
            ),
            if (manageable)
              const NfOption(
                collection,
                'Add to collection',
                leading: Icon(PhosphorIconsRegular.books, size: 18),
              ),
            if (!work.isContainer && !linked)
              const NfOption(
                room,
                'Start a watch room',
                leading: Icon(PhosphorIconsRegular.broadcast, size: 18),
              ),
            if (exportable) ...[
              const NfOption(
                exportMp4,
                'Make an offline copy (MP4)',
                leading: Icon(PhosphorIconsRegular.downloadSimple, size: 18),
              ),
              const NfOption(
                exportMkv,
                'Make an offline copy (MKV)',
                leading: Icon(PhosphorIconsRegular.downloadSimple, size: 18),
              ),
            ],
            if (manageable)
              const NfOption(
                manage,
                'Edit, identify & manage…',
                leading: Icon(PhosphorIconsRegular.pencilSimple, size: 18),
              ),
          ],
        );
        Future<void> makeCopy(GExportFormat format) async {
          final edition = controller.edition.value;
          if (edition == null) return;
          await getIt<DownloadsService>().createExport(
            edition.id,
            server: controller.server,
            format: format,
          );
        }

        switch (choice) {
          case playlist:
            await _addToPlaylist(work, controller.server);
          case collection:
            await _addToCollection(work);
          case room:
            await _createRoom(controller);
          case exportMp4:
            await makeCopy(GExportFormat.MP4);
          case exportMkv:
            await makeCopy(GExportFormat.MKV);
          case manage:
            await openManageSheet(controller);
        }
      },
    );
  }

  static Future<void> _addToCollection(WorkSummary work) async {
    final graphql = getIt<GraphqlService>();
    final toasts = getIt<ToastService>();
    final menu = await toasts.attempt(
      () => graphql.run(
        GCollectionMenuReq((b) => b.vars.library = work.libraryId),
      ),
    );
    if (menu == null) return;
    final collections = menu.collections.edges;
    if (collections.isEmpty) {
      toasts.show('No collections in this library yet.');
      return;
    }
    final choice = await pickOption<String>(
      title: 'Add to collection',
      options: [
        for (final edge in collections) NfOption(edge.node.id, edge.node.title),
      ],
    );
    if (choice == null) return;
    await toasts.attempt(() async {
      // The mutation replaces the whole list, so it needs all of it first.
      final ids = <String>[];
      String? after;
      do {
        final page = await graphql.run(
          GCollectionItemsReq(
            (b) => b
              ..fetchPolicy = FetchPolicy.NetworkOnly
              ..vars.id = choice
              ..vars.after = after,
          ),
        );
        final items = page.collection?.items;
        ids.addAll(items?.nodes.map((node) => node.id) ?? const []);
        after = items?.pageInfo.hasNextPage ?? false
            ? items?.pageInfo.endCursor
            : null;
      } while (after != null);
      if (ids.contains(work.id)) return;
      await graphql.run(
        GSetCollectionItemsReq(
          (b) => b.vars
            ..id = choice
            ..items = ListBuilder([...ids, work.id]),
        ),
      );
    }, success: 'Added to collection');
  }

  static Future<void> _addToPlaylist(
    WorkSummary work,
    ServerRef? server,
  ) async {
    final graphql = getIt<GraphqlService>();
    final toasts = getIt<ToastService>();
    final data = await toasts.attempt(
      () => graphql.run(GPlaylistMenuReq(), server: server),
    );
    if (data == null) return;
    if (data.playlists.nodes.isEmpty) {
      toasts.show('No playlists yet. Create one under Collections.');
      return;
    }
    final choice = await pickOption<String>(
      title: 'Add to playlist',
      options: [
        for (final list in data.playlists.nodes) NfOption(list.id, list.title),
      ],
    );
    if (choice == null) return;
    await toasts.attempt(
      () => graphql.run(
        GAddToPlaylistReq(
          (b) => b.vars
            ..id = choice
            ..items = ListBuilder([playTarget(work.id)]),
        ),
        server: server,
      ),
      success: 'Added to playlist',
    );
  }

  static Future<void> _createRoom(WorkDetailController controller) async {
    final version = controller.version.value;
    if (version == null) return;
    final data = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GCreateWatchRoomReq((b) => b.vars.media = version.id),
      ),
    );
    if (data == null) return;
    await getIt<NavService>().push(
      WatchRoomsScreen(roomId: data.createWatchRoom.id),
    );
  }
}

GPlayTargetInput playTarget(String workId) => GPlayTargetInput(
  (b) => b
    ..kind = GPlayTargetKind.WORK
    ..id = workId,
);
