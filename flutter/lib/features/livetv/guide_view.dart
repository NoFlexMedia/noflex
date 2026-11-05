// SPDX-License-Identifier: AGPL-3.0-only
//
// The guide: a time grid with room for one, a per-channel now and next list
// without. The grid pins its channel column and time header by keeping them in
// their own scrollables, linked to the lanes.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_state.dart';
import '../player/playback_service.dart';
import 'channel_logo.dart';
import 'guide_math.dart';
import 'live_tv_controller.dart';

final _time = DateFormat.jm();

String _range(GuideProgram program) =>
    '${_time.format(program.start.toLocal())} – '
    '${_time.format(program.end.toLocal())}';

/// Below this the grid can't show enough of a program to be worth its scrolling.
const _gridMinWidth = 600.0;

class GuideView extends StatelessWidget {
  const GuideView({super.key, required this.controller});
  final LiveTvController controller;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      SignalBuilder(
        builder: (context) {
          final failure = controller.guideError.value;
          if (failure != null && controller.programs.value.isEmpty) {
            return Padding(
              padding: EdgeInsets.symmetric(
                horizontal: context.gutter,
                vertical: 8,
              ),
              child: Row(
                children: [
                  const Icon(
                    PhosphorIconsRegular.warningCircle,
                    size: 16,
                    color: NfColors.warning,
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      'The guide didn’t load: $failure',
                      style: nfText(13, color: NfColors.warning),
                    ),
                  ),
                  const SizedBox(width: 8),
                  NfButton(
                    label: 'Retry',
                    style: NfButtonStyle.ghost,
                    size: NfButtonSize.small,
                    onPressed: () => unawaited(controller.loadGuide()),
                  ),
                ],
              ),
            );
          }
          return controller.guideLoading.value &&
                  controller.programs.value.isEmpty
              ? const NfProgressBar(value: null, height: 2)
              : const SizedBox.shrink();
        },
      ),
      Expanded(
        child: SignalBuilder(
          builder: (context) => controller.channels.value.isEmpty
              ? const NfState(
                  title: 'No channels right now',
                  message: 'Your recordings are still on the DVR tab.',
                )
              : LayoutBuilder(
                  builder: (context, constraints) =>
                      constraints.maxWidth < _gridMinWidth
                      ? _NowNextList(controller: controller)
                      : _GuideGrid(controller: controller),
                ),
        ),
      ),
    ],
  );
}

Future<void> openProgram(
  LiveTvController controller,
  GuideProgram program,
  LiveChannel channel,
) => getIt<NavService>().openSheet<void>(
  (_) =>
      _ProgramSheet(controller: controller, program: program, channel: channel),
);

class _ProgramSheet extends StatefulWidget {
  const _ProgramSheet({
    required this.controller,
    required this.program,
    required this.channel,
  });

  final LiveTvController controller;
  final GuideProgram program;
  final LiveChannel channel;

  @override
  State<_ProgramSheet> createState() => _ProgramSheetState();
}

class _ProgramSheetState extends State<_ProgramSheet> {
  final _recording = signal(false);

  Future<void> _record() async {
    _recording.value = true;
    final done = await widget.controller.record(widget.program);
    if (!mounted) return;
    _recording.value = false;
    if (done) Navigator.of(context).pop();
  }

  void _watch() {
    Navigator.of(context).pop();
    unawaited(
      getIt<PlaybackService>().playLive(
        channelId: widget.channel.id,
        name: widget.channel.name,
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final program = widget.program;
    final channel = widget.channel;
    final day = DateFormat.MMMEd().format(program.start.toLocal());
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.all(22),
        child: SignalBuilder(
          builder: (context) {
            final now = widget.controller.now.value;
            final on = program.isOn(now);
            final scheduled = widget.controller.scheduled.value.contains(
              program.id,
            );
            return Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(program.title, style: NfType.sectionTitle),
                if (program.subtitle?.isNotEmpty ?? false) ...[
                  const SizedBox(height: 4),
                  Text(program.subtitle!, style: NfType.bodyMuted),
                ],
                const SizedBox(height: 14),
                Row(
                  children: [
                    ChannelLogo(
                      name: channel.name,
                      logoUrl: channel.logoUrl,
                      size: 34,
                    ),
                    const SizedBox(width: 12),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(channel.name, style: nfText(14.5, weight: 600)),
                          Text(
                            '$day · ${_range(program)} · '
                            '${runtime(program.length.inMilliseconds)}',
                            style: NfType.caption,
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
                if (on) ...[
                  const SizedBox(height: 14),
                  Row(
                    children: [
                      const NfBadge(
                        'On now',
                        color: NfColors.danger,
                        background: NfColors.dangerSoft,
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: NfProgressBar(value: program.progress(now)),
                      ),
                      const SizedBox(width: 10),
                      Text(
                        '${_left(program, now)} left',
                        style: nfText(12.5, color: NfColors.text2),
                      ),
                    ],
                  ),
                ],
                if (program.categories.isNotEmpty) ...[
                  const SizedBox(height: 14),
                  Wrap(
                    spacing: 6,
                    runSpacing: 6,
                    children: [
                      for (final category in program.categories)
                        NfBadge(category),
                    ],
                  ),
                ],
                const SizedBox(height: 14),
                Flexible(
                  child: SingleChildScrollView(
                    child: Text(
                      program.description?.isNotEmpty ?? false
                          ? program.description!
                          : 'No description in the guide for this program.',
                      style: NfType.bodyMuted,
                    ),
                  ),
                ),
                const SizedBox(height: 20),
                Wrap(
                  alignment: WrapAlignment.end,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  spacing: 10,
                  runSpacing: 10,
                  children: [
                    if (scheduled)
                      Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          const Icon(
                            PhosphorIconsFill.record,
                            size: 14,
                            color: NfColors.danger,
                          ),
                          const SizedBox(width: 6),
                          Text('Recording scheduled', style: NfType.small),
                        ],
                      )
                    else if (!program.isOver(now))
                      NfButton(
                        label: 'Record',
                        style: NfButtonStyle.ghost,
                        icon: const Icon(PhosphorIconsFill.record),
                        busy: _recording.value,
                        onPressed: _record,
                      ),
                    NfButton(
                      label: on ? 'Watch live' : 'Watch channel',
                      style: NfButtonStyle.primary,
                      icon: const Icon(PhosphorIconsFill.play),
                      onPressed: _watch,
                    ),
                  ],
                ),
              ],
            );
          },
        ),
      ),
    );
  }
}

String _left(GuideProgram program, DateTime now) {
  final left = program.end.difference(now).inMilliseconds;
  return left < Duration.millisecondsPerMinute ? '<1m' : runtime(left);
}

// ---- compact ------------------------------------------------------------

class _NowNextList extends StatelessWidget {
  const _NowNextList({required this.controller});
  final LiveTvController controller;

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final channels = controller.visibleChannels.value;
      return ListView.separated(
        padding: EdgeInsets.fromLTRB(context.gutter, 6, context.gutter, 32),
        itemCount: channels.length,
        separatorBuilder: (_, _) => const SizedBox(height: 10),
        itemBuilder: (_, index) =>
            _NowNextCard(controller: controller, channel: channels[index]),
      );
    },
  );
}

class _NowNextCard extends SignalWidget {
  const _NowNextCard({required this.controller, required this.channel});
  final LiveTvController controller;
  final LiveChannel channel;

  @override
  Widget build(BuildContext context) {
    final playback = getIt<PlaybackService>();
    final now = controller.now.value;
    final programs =
        controller.programsByChannel.value[channel.id] ??
        const <GuideProgram>[];
    final (:current, :next) = nowAndNext(programs, now);
    final starting = playback.preparing.value == channel.id;
    return NfPanel(
      padding: EdgeInsets.zero,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          InkWell(
            onTap: starting
                ? null
                : () => playback.playLive(
                    channelId: channel.id,
                    name: channel.name,
                  ),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
              child: Row(
                children: [
                  ChannelLogo(
                    name: channel.name,
                    logoUrl: channel.logoUrl,
                    size: 36,
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Text(
                      [?channel.numberLabel, channel.name].join('  '),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: nfText(14.5, weight: 600),
                    ),
                  ),
                  if (starting)
                    const SizedBox.square(
                      dimension: 18,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  else
                    const Icon(
                      PhosphorIconsFill.play,
                      size: 15,
                      color: NfColors.text2,
                    ),
                ],
              ),
            ),
          ),
          const Divider(height: 1),
          if (current == null && next == null)
            Padding(
              padding: const EdgeInsets.all(12),
              child: Text('No guide information', style: NfType.caption),
            )
          else ...[
            if (current != null)
              _NowNextLine(
                label: 'Now',
                program: current,
                progress: current.progress(now),
                scheduled: controller.scheduled.value.contains(current.id),
                onTap: () => openProgram(controller, current, channel),
              ),
            if (next != null)
              _NowNextLine(
                label: _time.format(next.start.toLocal()),
                program: next,
                scheduled: controller.scheduled.value.contains(next.id),
                onTap: () => openProgram(controller, next, channel),
              ),
          ],
        ],
      ),
    );
  }
}

class _NowNextLine extends StatelessWidget {
  const _NowNextLine({
    required this.label,
    required this.program,
    required this.onTap,
    required this.scheduled,
    this.progress,
  });

  final String label;
  final GuideProgram program;
  final VoidCallback onTap;
  final bool scheduled;
  final double? progress;

  @override
  Widget build(BuildContext context) => InkWell(
    onTap: onTap,
    child: Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              SizedBox(
                width: 66,
                child: Text(
                  label,
                  style: nfText(
                    12,
                    weight: 600,
                    color: progress != null ? NfColors.danger : NfColors.text3,
                  ),
                ),
              ),
              if (scheduled)
                const Padding(
                  padding: EdgeInsets.only(right: 6),
                  child: Icon(
                    PhosphorIconsFill.record,
                    size: 10,
                    color: NfColors.danger,
                  ),
                ),
              Expanded(
                child: Text(
                  program.title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: nfText(13.5, weight: 500),
                ),
              ),
            ],
          ),
          if (progress != null) ...[
            const SizedBox(height: 6),
            NfProgressBar(value: progress, height: 3),
          ],
        ],
      ),
    ),
  );
}

// ---- grid ---------------------------------------------------------------

class _GuideGrid extends StatefulWidget {
  const _GuideGrid({required this.controller});
  final LiveTvController controller;

  @override
  State<_GuideGrid> createState() => _GuideGridState();
}

class _GuideGridState extends State<_GuideGrid> {
  final _laneAcross = ScrollController();
  final _headerAcross = ScrollController();
  final _laneDown = ScrollController();
  final _channelsDown = ScrollController();
  bool _following = false;
  double _pixelsPerMinute = 6;

  LiveTvController get _controller => widget.controller;

  @override
  void initState() {
    super.initState();
    _link(_laneAcross, _headerAcross);
    _link(_laneDown, _channelsDown);
    WidgetsBinding.instance.addPostFrameCallback((_) => _jumpToNow());
  }

  /// Each drives the other; a clamped position has no overscroll for them to
  /// fight over.
  void _link(ScrollController a, ScrollController b) {
    void follow(ScrollController from, ScrollController to) {
      if (_following || !from.hasClients || !to.hasClients) return;
      if (from.offset == to.offset) return;
      _following = true;
      to.jumpTo(from.offset);
      _following = false;
    }

    a.addListener(() => follow(a, b));
    b.addListener(() => follow(b, a));
  }

  double get _nowOffset {
    final minutes = guideNowMinutes(
      _controller.anchor.value,
      _controller.windowEnd.value,
      _controller.now.value,
    );
    return math.max(0, ((minutes ?? 0) - 20) * _pixelsPerMinute);
  }

  void _jumpToNow() {
    if (_laneAcross.hasClients) _laneAcross.jumpTo(_clamped(_nowOffset));
  }

  double _clamped(double offset) =>
      math.min(offset, _laneAcross.position.maxScrollExtent);

  void _scrollToNow() {
    if (!_laneAcross.hasClients) return;
    unawaited(
      _laneAcross.animateTo(
        _clamped(_nowOffset),
        duration: NfMotion.slow,
        curve: NfMotion.easeOut,
      ),
    );
  }

  @override
  void dispose() {
    _laneAcross.dispose();
    _headerAcross.dispose();
    _laneDown.dispose();
    _channelsDown.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final roomy = constraints.maxWidth >= 760;
      _pixelsPerMinute = roomy ? 6 : 4.4;
      final channelWidth = roomy ? 208.0 : 132.0;
      final rowHeight = roomy ? 70.0 : 62.0;
      final canvasWidth = guideSpan.inMinutes * _pixelsPerMinute;
      const physics = ClampingScrollPhysics();
      return Padding(
        padding: EdgeInsets.fromLTRB(
          context.gutter / 2,
          4,
          context.gutter / 2,
          12,
        ),
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: NfColors.bgRaised,
            borderRadius: BorderRadius.circular(NfRadii.l),
            border: Border.all(color: NfColors.line),
          ),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(NfRadii.l),
            // The scrollbars below are placed by hand, since a vertical list
            // inside a horizontal one would otherwise have its bar off-screen.
            // A mouse can drag this too: it's the one surface here that scrolls
            // in both directions.
            child: ScrollConfiguration(
              behavior: ScrollConfiguration.of(context).copyWith(
                scrollbars: false,
                dragDevices: {...PointerDeviceKind.values},
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    height: 46,
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Container(
                          width: channelWidth,
                          padding: const EdgeInsets.symmetric(horizontal: 10),
                          alignment: Alignment.centerLeft,
                          decoration: const BoxDecoration(
                            border: Border(
                              right: BorderSide(color: NfColors.line),
                            ),
                          ),
                          child: NfButton(
                            label: 'Now',
                            size: NfButtonSize.small,
                            style: NfButtonStyle.glass,
                            icon: const Icon(PhosphorIconsRegular.clock),
                            onPressed: _scrollToNow,
                          ),
                        ),
                        Expanded(
                          child: SingleChildScrollView(
                            controller: _headerAcross,
                            scrollDirection: Axis.horizontal,
                            physics: physics,
                            child: SizedBox(
                              width: canvasWidth,
                              child: _TimeHeader(
                                controller: _controller,
                                pixelsPerMinute: _pixelsPerMinute,
                              ),
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                  const Divider(height: 1),
                  Expanded(
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        SizedBox(
                          width: channelWidth,
                          child: SignalBuilder(
                            builder: (context) {
                              final channels =
                                  _controller.visibleChannels.value;
                              return ListView.builder(
                                controller: _channelsDown,
                                physics: physics,
                                itemExtent: rowHeight,
                                itemCount: channels.length,
                                itemBuilder: (_, index) => _ChannelCell(
                                  channel: channels[index],
                                  compact: !roomy,
                                ),
                              );
                            },
                          ),
                        ),
                        Expanded(
                          child: Scrollbar(
                            controller: _laneDown,
                            notificationPredicate: (n) => n.depth == 1,
                            child: Scrollbar(
                              controller: _laneAcross,
                              notificationPredicate: (n) => n.depth == 0,
                              child: SingleChildScrollView(
                                controller: _laneAcross,
                                scrollDirection: Axis.horizontal,
                                physics: physics,
                                child: SizedBox(
                                  width: canvasWidth,
                                  child: Stack(
                                    fit: StackFit.expand,
                                    children: [
                                      SignalBuilder(
                                        builder: (context) {
                                          final channels =
                                              _controller.visibleChannels.value;
                                          return ListView.builder(
                                            controller: _laneDown,
                                            physics: physics,
                                            itemExtent: rowHeight,
                                            itemCount: channels.length,
                                            itemBuilder: (_, index) => _Lane(
                                              controller: _controller,
                                              channel: channels[index],
                                              pixelsPerMinute: _pixelsPerMinute,
                                              across: _laneAcross,
                                            ),
                                          );
                                        },
                                      ),
                                      _NowLine(
                                        controller: _controller,
                                        pixelsPerMinute: _pixelsPerMinute,
                                      ),
                                    ],
                                  ),
                                ),
                              ),
                            ),
                          ),
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
    },
  );
}

class _TimeHeader extends SignalWidget {
  const _TimeHeader({required this.controller, required this.pixelsPerMinute});
  final LiveTvController controller;
  final double pixelsPerMinute;

  @override
  Widget build(BuildContext context) {
    final start = controller.anchor.value;
    final end = controller.windowEnd.value;
    final nowMinutes = guideNowMinutes(start, end, controller.now.value);
    final weekday = DateFormat.E();
    return Stack(
      children: [
        for (final slot in guideSlots(start, end))
          Positioned(
            left: slot.minutes * pixelsPerMinute,
            top: 0,
            bottom: 0,
            child: Container(
              padding: const EdgeInsets.only(left: 10),
              alignment: Alignment.center,
              decoration: const BoxDecoration(
                border: Border(left: BorderSide(color: NfColors.line)),
              ),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  if (slot.startsDay) ...[
                    Text(
                      weekday.format(slot.at.toLocal()),
                      style: nfText(12.5, weight: 700, color: NfColors.accent),
                    ),
                    const SizedBox(width: 6),
                  ],
                  Text(
                    _time.format(slot.at.toLocal()),
                    style: nfText(12.5, weight: 600, color: NfColors.text3),
                  ),
                ],
              ),
            ),
          ),
        if (nowMinutes != null)
          Positioned(
            left: nowMinutes * pixelsPerMinute,
            bottom: 5,
            child: FractionalTranslation(
              translation: const Offset(-0.5, 0),
              child: Container(
                padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
                decoration: BoxDecoration(
                  color: NfColors.danger,
                  borderRadius: BorderRadius.circular(NfRadii.pill),
                ),
                child: Text(
                  _time.format(controller.now.value.toLocal()),
                  style: nfText(
                    11,
                    weight: 700,
                    color: NfColors.accentInk,
                    features: NfType.tabular,
                  ),
                ),
              ),
            ),
          ),
      ],
    );
  }
}

class _NowLine extends SignalWidget {
  const _NowLine({required this.controller, required this.pixelsPerMinute});
  final LiveTvController controller;
  final double pixelsPerMinute;

  @override
  Widget build(BuildContext context) {
    final minutes = guideNowMinutes(
      controller.anchor.value,
      controller.windowEnd.value,
      controller.now.value,
    );
    if (minutes == null) return const SizedBox.shrink();
    return Stack(
      children: [
        Positioned(
          left: minutes * pixelsPerMinute - 1,
          top: 0,
          bottom: 0,
          width: 2,
          child: const IgnorePointer(child: ColoredBox(color: NfColors.danger)),
        ),
      ],
    );
  }
}

class _ChannelCell extends SignalWidget {
  const _ChannelCell({required this.channel, required this.compact});
  final LiveChannel channel;
  final bool compact;

  @override
  Widget build(BuildContext context) {
    final playback = getIt<PlaybackService>();
    final starting = playback.preparing.value == channel.id;
    return Semantics(
      button: true,
      label: 'Watch ${channel.name} live',
      excludeSemantics: true,
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          onTap: starting
              ? null
              : () => playback.playLive(
                  channelId: channel.id,
                  name: channel.name,
                ),
          child: Container(
            padding: EdgeInsets.symmetric(horizontal: compact ? 8 : 12),
            decoration: const BoxDecoration(
              border: Border(
                right: BorderSide(color: NfColors.line),
                bottom: BorderSide(color: NfColors.line),
              ),
            ),
            child: Row(
              children: [
                ChannelLogo(
                  name: channel.name,
                  logoUrl: channel.logoUrl,
                  size: compact ? 28 : 36,
                ),
                SizedBox(width: compact ? 8 : 11),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      Text(
                        channel.name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: nfText(compact ? 12.5 : 14, weight: 600),
                      ),
                      if (channel.numberLabel case final number?)
                        Text(number, style: NfType.caption),
                    ],
                  ),
                ),
                if (starting)
                  const SizedBox.square(
                    dimension: 16,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _Lane extends SignalWidget {
  const _Lane({
    required this.controller,
    required this.channel,
    required this.pixelsPerMinute,
    required this.across,
  });

  final LiveTvController controller;
  final LiveChannel channel;
  final double pixelsPerMinute;
  final ScrollController across;

  @override
  Widget build(BuildContext context) {
    final blocks = controller.rows.value[channel.id] ?? const <GuideBlock>[];
    final now = controller.now.value;
    final scheduled = controller.scheduled.value;
    final nowMinutes = guideNowMinutes(
      controller.anchor.value,
      controller.windowEnd.value,
      now,
    );
    return DecoratedBox(
      decoration: const BoxDecoration(
        border: Border(bottom: BorderSide(color: NfColors.line)),
      ),
      child: Stack(
        children: [
          for (final block in blocks)
            Positioned(
              left: block.startMinutes * pixelsPerMinute + 2,
              width: math.max(block.lengthMinutes * pixelsPerMinute - 4, 2),
              top: 6,
              bottom: 6,
              child: _ProgramTile(
                program: block.program,
                channel: channel,
                now: now,
                narrow: block.lengthMinutes < 20,
                scheduled: scheduled.contains(block.program.id),
                across: across,
                left: block.startMinutes * pixelsPerMinute + 2,
                onTap: () => openProgram(controller, block.program, channel),
              ),
            ),
          if (blocks.isEmpty && !controller.guideLoading.value)
            Positioned(
              left: (nowMinutes ?? 0) * pixelsPerMinute + 14,
              top: 0,
              bottom: 0,
              child: Center(
                child: Text('No guide data', style: NfType.caption),
              ),
            ),
        ],
      ),
    );
  }
}

class _ProgramTile extends StatelessWidget {
  const _ProgramTile({
    required this.program,
    required this.channel,
    required this.now,
    required this.narrow,
    required this.scheduled,
    required this.across,
    required this.left,
    required this.onTap,
  });

  final GuideProgram program;
  final LiveChannel channel;
  final DateTime now;
  final bool narrow;
  final bool scheduled;
  final ScrollController across;

  /// Where this tile starts along the lane.
  final double left;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final on = program.isOn(now);
    return Semantics(
      button: true,
      label: [
        program.title,
        channel.name,
        _range(program),
        if (on) 'on now',
        if (scheduled) 'recording scheduled',
      ].join(', '),
      excludeSemantics: true,
      child: Opacity(
        opacity: program.isOver(now) ? 0.45 : 1,
        child: Material(
          color: on ? NfColors.surface : NfColors.surface2,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(NfRadii.s),
            side: on
                ? const BorderSide(color: NfColors.accentSoft)
                : BorderSide.none,
          ),
          clipBehavior: Clip.antiAlias,
          child: InkWell(
            onTap: onTap,
            child: Stack(
              fit: StackFit.expand,
              children: [
                if (on)
                  FractionallySizedBox(
                    alignment: Alignment.centerLeft,
                    widthFactor: program.progress(now),
                    child: const ColoredBox(color: NfColors.accentSoft),
                  ),
                CustomSingleChildLayout(
                  delegate: _TitleInView(across: across, left: left),
                  child: Padding(
                    padding: EdgeInsets.symmetric(horizontal: narrow ? 6 : 12),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Row(
                          children: [
                            if (scheduled)
                              const Padding(
                                padding: EdgeInsets.only(right: 5),
                                child: Icon(
                                  PhosphorIconsFill.record,
                                  size: 9,
                                  color: NfColors.danger,
                                ),
                              ),
                            Expanded(
                              child: Text(
                                program.title,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                softWrap: false,
                                style: nfText(13.5, weight: 600),
                              ),
                            ),
                          ],
                        ),
                        if (!narrow)
                          Text(
                            _range(program),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            softWrap: false,
                            style: nfText(
                              11.5,
                              color: on ? NfColors.accent : NfColors.text3,
                              features: NfType.tabular,
                            ),
                          ),
                      ],
                    ),
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

/// Keeps a program's title in view when it began before the visible part of the
/// guide, or a long film would show no name for most of the time it's on. Done
/// in layout so scrolling doesn't rebuild every program in every visible lane.
class _TitleInView extends SingleChildLayoutDelegate {
  _TitleInView({required this.across, required this.left})
    : super(relayout: across);

  final ScrollController across;
  final double left;

  /// Room a title keeps as its program scrolls away, rather than a sliver.
  static const _minWidth = 88.0;

  double _inset(double width) {
    final hidden = across.hasClients ? across.offset - left : 0.0;
    return hidden.clamp(0.0, math.max(0.0, width - _minWidth));
  }

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints constraints) =>
      BoxConstraints.tightFor(
        width: constraints.maxWidth - _inset(constraints.maxWidth),
        height: constraints.maxHeight,
      );

  @override
  Offset getPositionForChild(Size size, Size childSize) =>
      Offset(_inset(size.width), 0);

  @override
  bool shouldRelayout(_TitleInView oldDelegate) =>
      oldDelegate.across != across || oldDelegate.left != left;
}
