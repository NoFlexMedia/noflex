// SPDX-License-Identifier: AGPL-3.0-only
//
// The picture and its chrome. On a foldable bent into tabletop posture the
// picture takes the upper half and the controls the lower, so nothing sits on
// the crease and the viewer's hands stay away from the screen they're watching.

import 'dart:async';
import 'dart:io';

import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:media_kit_video/media_kit_video.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../services/tokens.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../utils/images.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import 'decoder_probe.dart';
import 'decoders.dart';
import 'playback_service.dart';
import 'player_controller.dart';

bool get _isDesktop =>
    Platform.isLinux || Platform.isMacOS || Platform.isWindows;

class PlayerScreen extends StatefulWidget {
  const PlayerScreen({super.key});

  @override
  State<PlayerScreen> createState() => _PlayerScreenState();
}

class _PlayerScreenState extends State<PlayerScreen> {
  late final PlayerController _player = PlayerController();
  final _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    if (!_isDesktop) {
      unawaited(_player.enterMobileFullscreen());
      unawaited(_lockOrientation());
    }
  }

  /// A phone turns sideways for a landscape picture; a tablet or an unfolded
  /// foldable is fine however it's being held.
  Future<void> _lockOrientation() async {
    final view = WidgetsBinding.instance.platformDispatcher.views.first;
    final shortest = view.physicalSize.shortestSide / view.devicePixelRatio;
    if (shortest < 600) {
      await SystemChrome.setPreferredOrientations(const [
        DeviceOrientation.landscapeLeft,
        DeviceOrientation.landscapeRight,
      ]);
    }
  }

  @override
  void dispose() {
    _focus.dispose();
    _player.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final fold = context.fold;
    final tabletop = fold != null && !fold.isVertical && fold.halfOpen;
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) unawaited(_player.close());
      },
      child: Focus(
        focusNode: _focus,
        autofocus: true,
        onKeyEvent: (_, event) => _player.onKey(event),
        child: Scaffold(
          backgroundColor: Colors.black,
          body: tabletop
              ? Column(
                  children: [
                    SizedBox(
                      height: fold.bounds.top,
                      child: _Picture(player: _player, chrome: false),
                    ),
                    SizedBox(height: fold.bounds.height),
                    Expanded(
                      child: ColoredBox(
                        color: NfColors.bg,
                        child: _Chrome(player: _player, docked: true),
                      ),
                    ),
                  ],
                )
              : _Picture(player: _player, chrome: true),
        ),
      ),
    );
  }
}

class _Picture extends StatelessWidget {
  const _Picture({required this.player, required this.chrome});

  final PlayerController player;
  final bool chrome;

  @override
  Widget build(BuildContext context) => MouseRegion(
    onHover: (_) => player.showChrome(),
    child: Stack(
      fit: StackFit.expand,
      children: [
        Video(
          controller: player.video,
          controls: NoVideoControls,
          fill: Colors.black,
          // libmpv draws subtitles into the picture through libass.
          subtitleViewConfiguration: const SubtitleViewConfiguration(
            visible: false,
          ),
        ),
        _Gestures(player: player),
        _Status(player: player),
        _Flash(player: player),
        if (chrome) _Chrome(player: player, docked: false),
        _InfoPanel(player: player),
      ],
    ),
  );
}

/// Tap for chrome; on touch, double-tap a side to skip. Desktop clicks toggle
/// playback the way every desktop player does.
class _Gestures extends StatelessWidget {
  const _Gestures({required this.player});
  final PlayerController player;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) => GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: _isDesktop ? player.toggle : player.toggleChrome,
      onDoubleTapDown: (details) {
        if (_isDesktop) {
          unawaited(player.toggleFullscreen());
          return;
        }
        final back = details.localPosition.dx < constraints.maxWidth / 2;
        player.skip(back ? -10000 : 10000);
        player.pulse(
          back
              ? PhosphorIconsRegular.arrowCounterClockwise
              : PhosphorIconsRegular.arrowClockwise,
        );
      },
      onDoubleTap: () {},
    ),
  );
}

class _Status extends SignalWidget {
  const _Status({required this.player});
  final PlayerController player;

  @override
  Widget build(BuildContext context) {
    final problem = player.error.value;
    if (problem.isNotEmpty) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(
              PhosphorIconsRegular.warningCircle,
              size: 34,
              color: NfColors.danger,
            ),
            const SizedBox(height: 10),
            Text(problem, style: NfType.body),
            const SizedBox(height: 14),
            NfButton(label: 'Close', onPressed: player.close),
          ],
        ),
      );
    }
    if (!player.waiting.value) return const SizedBox.shrink();
    return const IgnorePointer(
      child: Center(child: CircularProgressIndicator(color: NfColors.text)),
    );
  }
}

class _Flash extends SignalWidget {
  const _Flash({required this.player});
  final PlayerController player;

  @override
  Widget build(BuildContext context) {
    final icon = player.flash.value;
    return IgnorePointer(
      child: Center(
        child: AnimatedOpacity(
          opacity: icon == null ? 0 : 1,
          duration: NfMotion.fast,
          child: Container(
            width: 72,
            height: 72,
            decoration: const BoxDecoration(
              color: Color(0x8C000000),
              shape: BoxShape.circle,
            ),
            child: icon == null
                ? null
                : Icon(icon, size: 30, color: NfColors.text),
          ),
        ),
      ),
    );
  }
}

class _Chrome extends SignalWidget {
  const _Chrome({required this.player, required this.docked});

  final PlayerController player;

  /// In tabletop posture the controls have their own half of the screen and
  /// stay visible.
  final bool docked;

  @override
  Widget build(BuildContext context) {
    final visible = docked || player.chromeVisible.value;
    final current = getIt<PlaybackService>().current.value;
    if (current == null) return const SizedBox.shrink();
    return IgnorePointer(
      ignoring: !visible,
      child: AnimatedOpacity(
        opacity: visible ? 1 : 0,
        duration: NfMotion.base,
        child: DecoratedBox(
          decoration: BoxDecoration(
            gradient: docked
                ? null
                : const LinearGradient(
                    begin: Alignment.topCenter,
                    end: Alignment.bottomCenter,
                    stops: [0, 0.22, 0.62, 1],
                    colors: [
                      Color(0xB3000000),
                      Colors.transparent,
                      Colors.transparent,
                      Color(0xD9000000),
                    ],
                  ),
          ),
          child: SafeArea(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
              child: Column(
                children: [
                  _TopBar(player: player, current: current),
                  Expanded(
                    child: Center(child: _Transport(player: player)),
                  ),
                  _SkipAndNext(player: player),
                  _Scrubber(player: player, current: current),
                  _BottomBar(player: player, current: current),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _TopBar extends StatelessWidget {
  const _TopBar({required this.player, required this.current});

  final PlayerController player;
  final ActivePlayback current;

  @override
  Widget build(BuildContext context) => Row(
    children: [
      NfIconButton(
        icon: PhosphorIconsRegular.arrowLeft,
        label: 'Close player',
        onPressed: player.close,
      ),
      const SizedBox(width: 4),
      Expanded(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              current.title,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: nfText(16, weight: 650),
            ),
            if (current.subtitle.isNotEmpty || current.room != null)
              Text(
                current.room != null
                    ? 'Watch room · ${current.room!.isHost ? 'you’re hosting' : 'synced to host'}'
                    : current.subtitle,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: NfType.small,
              ),
          ],
        ),
      ),
      NfIconButton(
        icon: PhosphorIconsRegular.info,
        label: 'Playback info',
        onPressed: player.toggleInfo,
      ),
    ],
  );
}

class _Transport extends SignalWidget {
  const _Transport({required this.player});
  final PlayerController player;

  @override
  Widget build(BuildContext context) {
    if (player.isGuest.value) return const SizedBox.shrink();
    final live = getIt<PlaybackService>().current.value?.live ?? false;
    return Row(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        if (!live)
          NfIconButton(
            icon: PhosphorIconsRegular.arrowCounterClockwise,
            label: 'Back 10 seconds',
            size: 56,
            iconSize: 26,
            onPressed: () => player.skip(-10000),
          ),
        const SizedBox(width: 18),
        NfIconButton(
          icon: player.playing.value
              ? PhosphorIconsFill.pause
              : PhosphorIconsFill.play,
          label: player.playing.value ? 'Pause' : 'Play',
          size: 72,
          iconSize: 32,
          background: const Color(0x33FFFFFF),
          onPressed: player.toggle,
        ),
        const SizedBox(width: 18),
        if (!live)
          NfIconButton(
            icon: PhosphorIconsRegular.arrowClockwise,
            label: 'Forward 10 seconds',
            size: 56,
            iconSize: 26,
            onPressed: () => player.skip(10000),
          ),
      ],
    );
  }
}

class _SkipAndNext extends SignalWidget {
  const _SkipAndNext({required this.player});
  final PlayerController player;

  @override
  Widget build(BuildContext context) {
    final current = getIt<PlaybackService>().current.value;
    final intro = player.inIntro.value;
    final next = player.nextUp.value;
    final left = player.countdown.value;
    return Row(
      mainAxisAlignment: MainAxisAlignment.end,
      children: [
        if (intro)
          Padding(
            padding: const EdgeInsets.only(left: 8, bottom: 8),
            child: NfButton(
              label: 'Skip intro',
              style: NfButtonStyle.glass,
              icon: const Icon(PhosphorIconsRegular.skipForward),
              onPressed: () => player.seekTo(current!.edition!.introEndMs!),
            ),
          ),
        if (left != null && next != null) ...[
          Padding(
            padding: const EdgeInsets.only(left: 8, bottom: 8),
            child: NfButton(
              label: 'Cancel',
              style: NfButtonStyle.ghost,
              onPressed: player.cancelCountdown,
            ),
          ),
          Padding(
            padding: const EdgeInsets.only(left: 8, bottom: 8),
            child: NfButton(
              label: 'Next in $left',
              style: NfButtonStyle.light,
              icon: const Icon(PhosphorIconsFill.skipForward),
              onPressed: player.playNext,
            ),
          ),
        ],
      ],
    );
  }
}

class _Scrubber extends SignalWidget {
  const _Scrubber({required this.player, required this.current});

  final PlayerController player;
  final ActivePlayback current;

  @override
  Widget build(BuildContext context) {
    final length = player.duration.value;
    final position = player.position.value;
    if (current.live) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 10),
        child: Row(
          children: [
            Container(
              width: 8,
              height: 8,
              decoration: const BoxDecoration(
                color: NfColors.danger,
                shape: BoxShape.circle,
              ),
            ),
            const SizedBox(width: 8),
            Text('LIVE', style: nfText(12, weight: 700, letterSpacing: 1)),
          ],
        ),
      );
    }
    double fraction(int ms) => length > 0 ? (ms / length).clamp(0.0, 1.0) : 0;
    return LayoutBuilder(
      builder: (context, constraints) {
        int at(double dx) =>
            ((dx / constraints.maxWidth).clamp(0.0, 1.0) * length).round();
        return Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (player.scrubbing.value)
              _ScrubPreview(
                player: player,
                current: current,
                ms: player.scrubTarget.value,
                maxWidth: constraints.maxWidth,
                fraction: fraction(player.scrubTarget.value),
              ),
            GestureDetector(
              behavior: HitTestBehavior.opaque,
              onHorizontalDragStart: (details) =>
                  player.startScrub(at(details.localPosition.dx)),
              onHorizontalDragUpdate: (details) =>
                  player.moveScrub(at(details.localPosition.dx)),
              onHorizontalDragEnd: (_) => player.endScrub(),
              onTapUp: (details) => player.seekTo(at(details.localPosition.dx)),
              child: SizedBox(
                height: 28,
                child: Center(
                  child: Stack(
                    clipBehavior: Clip.none,
                    alignment: Alignment.centerLeft,
                    children: [
                      Container(
                        height: 4,
                        decoration: BoxDecoration(
                          color: const Color(0x38FFFFFF),
                          borderRadius: BorderRadius.circular(2),
                        ),
                      ),
                      FractionallySizedBox(
                        widthFactor: fraction(player.buffered.value),
                        child: Container(
                          height: 4,
                          decoration: BoxDecoration(
                            color: const Color(0x52FFFFFF),
                            borderRadius: BorderRadius.circular(2),
                          ),
                        ),
                      ),
                      FractionallySizedBox(
                        widthFactor: fraction(position),
                        child: Container(
                          height: 4,
                          decoration: BoxDecoration(
                            color: NfColors.accent,
                            borderRadius: BorderRadius.circular(2),
                          ),
                        ),
                      ),
                      Positioned(
                        left: fraction(position) * constraints.maxWidth - 7,
                        child: Container(
                          width: 14,
                          height: 14,
                          decoration: const BoxDecoration(
                            color: NfColors.accent,
                            shape: BoxShape.circle,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}

/// The frame under a scrub, cut from the server's trickplay sheets.
class _ScrubPreview extends StatelessWidget {
  const _ScrubPreview({
    required this.player,
    required this.current,
    required this.ms,
    required this.maxWidth,
    required this.fraction,
  });

  final PlayerController player;
  final ActivePlayback current;
  final int ms;
  final double maxWidth;
  final double fraction;

  @override
  Widget build(BuildContext context) {
    final trick = current.edition?.trickplay;
    Widget? frame;
    if (trick != null && trick.sheets.isNotEmpty && trick.intervalMs > 0) {
      final perSheet = trick.tileColumns * trick.tileRows;
      final index = ms ~/ trick.intervalMs;
      final sheetIndex = index ~/ perSheet;
      final sheet = trick.sheets
          .where((s) => s.index == sheetIndex)
          .firstOrNull;
      if (sheet != null) {
        final tile = index % perSheet;
        frame = SpriteThumb(
          url: sheet.url,
          server: current.server?.origin,
          x: (tile % trick.tileColumns) * trick.thumbWidth,
          y: (tile ~/ trick.tileColumns) * trick.thumbHeight,
          width: trick.thumbWidth,
          height: trick.thumbHeight,
          sheetWidth: trick.tileColumns * trick.thumbWidth,
          sheetHeight: trick.tileRows * trick.thumbHeight,
          displayWidth: 176,
        );
      }
    }
    const previewWidth = 176.0;
    final left = (fraction * maxWidth - previewWidth / 2).clamp(
      0.0,
      maxWidth - previewWidth,
    );
    return Padding(
      padding: EdgeInsets.only(left: left, bottom: 6),
      child: Align(
        alignment: Alignment.centerLeft,
        child: SizedBox(
          width: previewWidth,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ?frame,
              const SizedBox(height: 4),
              Text(
                timecode(ms),
                style: nfText(13, weight: 650, features: NfType.tabular),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class SpriteThumb extends StatelessWidget {
  const SpriteThumb({
    super.key,
    required this.url,
    required this.x,
    required this.y,
    required this.width,
    required this.height,
    required this.displayWidth,
    this.sheetWidth,
    this.sheetHeight,
    this.server,
  });

  final String url;
  final String? server;
  final int x;
  final int y;
  final int width;
  final int height;
  final int? sheetWidth;
  final int? sheetHeight;
  final double displayWidth;

  @override
  Widget build(BuildContext context) {
    final absolute = url.startsWith('/')
        ? (server != null ? '$server$url' : getIt<TokenStore>().absolute(url))
        : url;
    final scale = displayWidth / width;
    return ClipRRect(
      borderRadius: BorderRadius.circular(NfRadii.s),
      child: SizedBox(
        width: displayWidth,
        height: height * scale,
        child: Stack(
          clipBehavior: Clip.hardEdge,
          children: [
            Positioned(
              left: -x * scale,
              top: -y * scale,
              width: sheetWidth == null ? null : sheetWidth! * scale,
              height: sheetHeight == null ? null : sheetHeight! * scale,
              child: CachedNetworkImage(
                imageUrl: absolute,
                cacheKey: imageCacheKey(absolute),
                fit: BoxFit.fill,
                placeholder: (_, _) =>
                    const ColoredBox(color: NfColors.surface2),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _BottomBar extends SignalWidget {
  const _BottomBar({required this.player, required this.current});

  final PlayerController player;
  final ActivePlayback current;

  @override
  Widget build(BuildContext context) {
    final length = player.duration.value;
    final position = player.position.value;
    final audio = current.version?.probe.audio ?? const [];
    final hasSubtitles =
        current.subtitleTracks.isNotEmpty ||
        player.embeddedSubtitles.value.isNotEmpty;
    return Row(
      children: [
        if (!current.live)
          Text(
            '${timecode(position)} / ${timecode(length)}',
            style: nfText(
              13,
              weight: 550,
              color: NfColors.text2,
              features: NfType.tabular,
            ),
          ),
        const Spacer(),
        if (player.nextUp.value != null && player.countdown.value == null)
          NfIconButton(
            icon: PhosphorIconsFill.skipForward,
            label: 'Next episode',
            onPressed: player.playNext,
          ),
        if (audio.length > 1)
          NfIconButton(
            icon: PhosphorIconsRegular.translate,
            label: 'Audio track',
            onPressed: () => _chooseAudio(player, audio),
          ),
        if (hasSubtitles)
          NfIconButton(
            icon: player.subtitleIndex.value != null
                ? PhosphorIconsFill.subtitles
                : PhosphorIconsRegular.subtitles,
            label: 'Subtitles',
            onPressed: () => _chooseSubtitles(player, current),
          ),
        if (_isDesktop)
          NfIconButton(
            icon: player.muted.value
                ? PhosphorIconsRegular.speakerSlash
                : PhosphorIconsRegular.speakerHigh,
            label: player.muted.value ? 'Unmute' : 'Mute',
            onPressed: player.toggleMute,
          ),
        NfIconButton(
          icon: player.fullscreen.value
              ? PhosphorIconsRegular.cornersIn
              : PhosphorIconsRegular.cornersOut,
          label: player.fullscreen.value ? 'Exit fullscreen' : 'Fullscreen',
          onPressed: player.toggleFullscreen,
        ),
      ],
    );
  }
}

Future<void> _chooseAudio(
  PlayerController player,
  List<dynamic> streams,
) async {
  final choice = await pickOption<int>(
    title: 'Audio',
    selected: player.audioIndex.value,
    options: [
      for (final stream in streams)
        NfOption(
          stream.index as int,
          (languageName(stream.language as String?).isNotEmpty
              ? languageName(stream.language as String?)
              : (stream.title as String?) ??
                    'Track ${(stream.index as int) + 1}'),
          detail: stream.format as String,
        ),
    ],
  );
  if (choice != null) await player.chooseAudio(choice);
}

Future<void> _chooseSubtitles(
  PlayerController player,
  ActivePlayback current,
) async {
  const off = -1;
  final choice = await pickOption<int>(
    title: 'Subtitles',
    selected: player.subtitleIndex.value ?? off,
    options: [
      const NfOption(off, 'Off'),
      for (final track in current.subtitleTracks)
        NfOption(
          track.index,
          track.label,
          detail: [
            if (track.forced) 'Forced',
            if (current.transcoding && track.isImage) 'Burned in',
          ].join(' · ').ifEmptyNull,
        ),
    ],
  );
  if (choice != null) {
    await player.chooseSubtitle(choice == off ? null : choice);
  }
}

extension on String {
  String? get ifEmptyNull => isEmpty ? null : this;
}

/// Why this is playing the way it is: the route, the plan, and what actually
/// decoded it, which is the first thing to check when a title stutters.
class _InfoPanel extends SignalWidget {
  const _InfoPanel({required this.player});
  final PlayerController player;

  static const _plans = {
    'DIRECT_PLAY': 'Direct play',
    'REMUX': 'Direct stream (remux)',
    'TRANSCODE_AUDIO': 'Transcoding audio',
    'TRANSCODE_VIDEO': 'Transcoding video',
    'TRANSCODE': 'Transcoding',
  };

  @override
  Widget build(BuildContext context) {
    if (!player.infoOpen.value) return const SizedBox.shrink();
    final current = getIt<PlaybackService>().current.value;
    final decision = current?.session?.decision;
    final stats = player.stats.value;
    final video = current?.version?.probe.video;
    final profile = getIt<DecoderProbeService>().profile.value;
    // A transcode's source says nothing about how its stream decodes.
    final tier =
        video == null || profile == null || (current?.transcoding ?? false)
        ? null
        : profile.tierOf(video);
    final rows = <(String, String)>[
      (
        'Playing',
        current?.localPath != null
            ? 'Downloaded file'
            : (_plans[decision?.plan.name] ?? decision?.plan.name ?? '—'),
      ),
      if (stats != null)
        (
          'Decoder',
          stats.hardware ? 'Hardware (${stats.decoder})' : 'Software',
        ),
      if (tier != null)
        (
          'Expected',
          switch (tier) {
            DecodeTier.hardware => 'Hardware',
            DecodeTier.software => 'Software',
            DecodeTier.server => 'Beyond this device',
          },
        ),
      if (video != null) ('Source', current!.version!.label ?? ''),
      if (decision?.target.videoCodec != null &&
          decision!.route.name != 'DIRECT')
        (
          'Target',
          '${codecName(decision.target.videoCodec?.name)} ${decision.target.width}×${decision.target.height}',
        ),
      if (decision?.encoder != null)
        (
          'Encoder',
          '${decision!.encoder}${decision.hardware ? ' (hardware)' : ''}',
        ),
      if (stats != null) ...[
        ('Picture', stats.size),
        ('Frame rate', stats.fps),
        ('Dropped', stats.dropped),
        ('Buffer', '${stats.bufferSeconds.toStringAsFixed(1)} s'),
      ],
      if (decision != null)
        for (final reason in decision.reasons) ('Why', reason.description),
    ];
    return Align(
      alignment: Alignment.topRight,
      child: SafeArea(
        child: Container(
          margin: const EdgeInsets.fromLTRB(16, 60, 16, 16),
          padding: const EdgeInsets.all(14),
          constraints: const BoxConstraints(maxWidth: 340),
          decoration: BoxDecoration(
            color: const Color(0xE60C0E12),
            borderRadius: BorderRadius.circular(NfRadii.m),
            border: Border.all(color: NfColors.lineStrong),
          ),
          child: SingleChildScrollView(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final (term, value) in rows)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 5),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        SizedBox(
                          width: 76,
                          child: Text(term, style: NfType.caption),
                        ),
                        Expanded(
                          child: Text(value, style: nfText(12.5, weight: 500)),
                        ),
                      ],
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
