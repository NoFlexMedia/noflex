// SPDX-License-Identifier: AGPL-3.0-only
//
// The foot of the sidebar, as on the web: the server's name and a dot, with a
// ring around the dot for each thing it's busy with. Each kind differs in shape
// and motion as well as color, so they stay apart without relying on hue.

import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../admin/admin_controller.dart';
import '../admin/admin_screen.dart';
import 'server_activity.dart';

const _menuWidth = 290.0;
const _pulseSize = 34.0;

/// All three motions come round together at this, so the loop doesn't jump.
const _loop = Duration(milliseconds: 79200);
const _scanOrbitSeconds = 1.8;
const _transcodeOrbitSeconds = 1.1;
const _breatheSeconds = 2.4;

Color _tone(ActivityKind kind) => switch (kind) {
  ActivityKind.scan => NfColors.focus,
  ActivityKind.transcode => NfColors.accent,
  ActivityKind.playing => NfColors.success,
  ActivityKind.optimize => NfColors.text2,
  ActivityKind.warn => NfColors.warning,
  ActivityKind.bad => NfColors.danger,
};

class ServerStatus extends StatefulWidget {
  const ServerStatus({super.key});

  @override
  State<ServerStatus> createState() => _ServerStatusState();
}

class _ServerStatusState extends State<ServerStatus>
    with SingleTickerProviderStateMixin {
  late final AnimationController _motion;
  ServerActivity? _activity;
  void Function()? _stopMotion;

  @override
  void initState() {
    super.initState();
    _motion = AnimationController(vsync: this, duration: _loop);
    if (getIt<SessionService>().viewer.value?.isAdmin ?? false) {
      final activity = _activity = ServerActivity();
      // Painting every frame is only worth it while there's a ring to move.
      _stopMotion = effect(() {
        final busy = activity.rings.value.isNotEmpty;
        untracked(() {
          if (busy && !_motion.isAnimating) {
            _motion.repeat();
          } else if (!busy) {
            _motion.stop();
          }
        });
      });
    }
  }

  @override
  void dispose() {
    _stopMotion?.call();
    _activity?.dispose();
    _motion.dispose();
    super.dispose();
  }

  void _openAdmin(MenuController menu, AdminSection? section) {
    menu.close();
    getIt<NavService>().open(
      NavPlace.administration,
      AdminScreen(initial: section),
      // A section was asked for, and an open screen wouldn't move to it.
      fresh: section != null,
    );
  }

  Widget _row({
    required bool collapsed,
    required List<ActivityEntry> problems,
    required List<ActivityEntry> rings,
    required String summary,
  }) {
    final name = getIt<SessionService>().serverName.value ?? 'NoFlex';
    return ConstrainedBox(
      constraints: const BoxConstraints(minHeight: 46),
      child: Padding(
        // On the rail's axis, with the text in line with the nav labels.
        padding: const EdgeInsets.fromLTRB(9, 4, 0, 4),
        child: Row(
          children: [
            SizedBox.square(
              dimension: _pulseSize,
              child: CustomPaint(
                painter: _PulsePainter(
                  rings: rings,
                  problem: problems.isEmpty ? null : problems.first.kind,
                  // Stopped, so a still picture rather than the frame it stopped on.
                  motion: MediaQuery.disableAnimationsOf(context)
                      ? const AlwaysStoppedAnimation(0)
                      : _motion,
                ),
              ),
            ),
            const SizedBox(width: 4),
            Expanded(
              child: AnimatedOpacity(
                opacity: collapsed ? 0 : 1,
                duration: NfMotion.fast,
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Text(
                      name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      softWrap: false,
                      style: nfText(12.6, weight: 600),
                    ),
                    Text(
                      summary,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      softWrap: false,
                      style: nfText(11.1, color: NfColors.text3),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final session = getIt<SessionService>();
      final name = session.serverName.value ?? 'NoFlex';
      final version = 'v${session.serverVersion.value}';
      // Read here: MenuAnchor's builder runs outside this tracking scope.
      final collapsed = getIt<NavService>().sidebarCollapsed.value;
      final activity = _activity;
      if (activity == null) {
        return Tooltip(
          message: '$name · $version',
          child: _row(
            collapsed: collapsed,
            problems: const [],
            rings: const [],
            summary: '$version · ${session.serverBackend.value}',
          ),
        );
      }
      final problems = activity.problems.value;
      final rings = activity.rings.value;
      final entries = [...problems, ...rings];
      final described = entries.isEmpty
          ? 'idle'
          : entries.map((entry) => entry.title).join(', ');
      return MenuAnchor(
        alignmentOffset: const Offset(8, 0),
        style: MenuStyle(
          alignment: AlignmentDirectional.topEnd,
          backgroundColor: const WidgetStatePropertyAll(NfColors.bgRaised),
          surfaceTintColor: const WidgetStatePropertyAll(Colors.transparent),
          padding: const WidgetStatePropertyAll(EdgeInsets.all(6)),
          shape: WidgetStatePropertyAll(
            RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(NfRadii.m),
              side: const BorderSide(color: NfColors.lineStrong),
            ),
          ),
        ),
        menuChildren: [
          _menuLabel('$name · $version'),
          for (final entry in entries) _menuEntry(entry),
          if (entries.isEmpty)
            SizedBox(
              width: _menuWidth,
              child: Padding(
                padding: const EdgeInsets.fromLTRB(11, 6, 11, 8),
                child: Text(
                  'Nothing is running right now.',
                  style: nfText(13.2, color: NfColors.text3),
                ),
              ),
            ),
          const Divider(height: 9),
          Builder(
            builder: (context) => _menuButton(
              onPressed: () =>
                  _openAdmin(MenuController.maybeOf(context)!, null),
              child: Text(
                'Open administration',
                style: nfText(13.8, weight: 500),
              ),
            ),
          ),
        ],
        builder: (context, menu, _) => Semantics(
          button: true,
          label: 'Server activity. $name: $described',
          excludeSemantics: true,
          child: Material(
            color: menu.isOpen ? NfColors.surface2 : Colors.transparent,
            borderRadius: BorderRadius.circular(11),
            clipBehavior: Clip.antiAlias,
            child: InkWell(
              hoverColor: NfColors.surface2,
              onTap: () => menu.isOpen ? menu.close() : menu.open(),
              child: _row(
                collapsed: collapsed,
                problems: problems,
                rings: rings,
                summary: activity.summary.value,
              ),
            ),
          ),
        ),
      );
    },
  );

  Widget _menuLabel(String text) => SizedBox(
    width: _menuWidth,
    child: Padding(
      padding: const EdgeInsets.fromLTRB(11, 7, 11, 6),
      child: Text(
        text,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: nfText(12, weight: 600, color: NfColors.text3),
      ),
    ),
  );

  Widget _menuEntry(ActivityEntry entry) {
    final body = Row(
      children: [
        SizedBox.square(
          dimension: 12,
          child: CustomPaint(painter: _SwatchPainter(entry)),
        ),
        const SizedBox(width: 11),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(entry.title, style: nfText(13.5, weight: 550)),
              Text(entry.detail, style: nfText(11.7, color: NfColors.text3)),
            ],
          ),
        ),
      ],
    );
    final section = entry.section;
    if (section == null) {
      return SizedBox(
        width: _menuWidth,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 7),
          child: body,
        ),
      );
    }
    return Builder(
      builder: (context) => _menuButton(
        onPressed: () => _openAdmin(MenuController.maybeOf(context)!, section),
        child: body,
      ),
    );
  }

  Widget _menuButton({
    required VoidCallback onPressed,
    required Widget child,
  }) => SizedBox(
    width: _menuWidth,
    child: Material(
      color: Colors.transparent,
      borderRadius: BorderRadius.circular(NfRadii.s),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        hoverColor: NfColors.surface2,
        onTap: onPressed,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 11, vertical: 7),
          child: child,
        ),
      ),
    ),
  );
}

class _PulsePainter extends CustomPainter {
  _PulsePainter({
    required this.rings,
    required this.problem,
    required this.motion,
  }) : super(repaint: motion);

  final List<ActivityEntry> rings;
  final ActivityKind? problem;
  final Animation<double> motion;

  @override
  void paint(Canvas canvas, Size size) {
    final center = size.center(Offset.zero);
    final seconds = motion.value * _loop.inMilliseconds / 1000;
    for (final (slot, ring) in rings.indexed) {
      paintRing(
        canvas,
        center,
        // Diameter 16, then 6 more for each ring out.
        8 + slot * 3.0,
        ring,
        seconds,
      );
    }
    final dot = problem == null ? NfColors.success : _tone(problem!);
    // The halo would run into the first ring.
    if (problem == null && rings.isEmpty) {
      canvas.drawCircle(center, 8, Paint()..color = const Color(0x246EE7A8));
    }
    canvas.drawCircle(center, 4, Paint()..color = dot);
  }

  @override
  bool shouldRepaint(_PulsePainter old) =>
      old.rings != rings || old.problem != problem || old.motion != motion;
}

/// The menu's key to the rings: the same shapes, at rest.
class _SwatchPainter extends CustomPainter {
  const _SwatchPainter(this.entry);
  final ActivityEntry entry;

  @override
  void paint(Canvas canvas, Size size) =>
      paintRing(canvas, size.center(Offset.zero), size.width / 2, entry, 0);

  @override
  bool shouldRepaint(_SwatchPainter old) => old.entry != entry;
}

/// A 2 px ring of [radius] (to its outer edge). [seconds] drives its motion.
void paintRing(
  Canvas canvas,
  Offset center,
  double radius,
  ActivityEntry ring,
  double seconds,
) {
  final tone = _tone(ring.kind);
  final rect = Rect.fromCircle(center: center, radius: radius - 1);
  final stroke = Paint()
    ..style = PaintingStyle.stroke
    ..strokeWidth = 2;
  const top = -math.pi / 2;
  const turn = math.pi * 2;
  void track(double alpha) => canvas.drawArc(
    rect,
    0,
    turn,
    false,
    stroke..color = tone.withValues(alpha: alpha),
  );
  switch (ring.kind) {
    case ActivityKind.scan:
      track(0.16);
      final start = top + turn * (seconds / _scanOrbitSeconds % 1);
      canvas.drawArc(rect, start, turn * 0.28, false, stroke..color = tone);
    case ActivityKind.transcode:
      final start = top + turn * (seconds / _transcodeOrbitSeconds % 1);
      stroke.color = tone;
      for (var dash = 0; dash < 4; dash++) {
        canvas.drawArc(
          rect,
          start + dash * turn / 4,
          turn * 0.16,
          false,
          stroke,
        );
      }
    case ActivityKind.playing:
      final phase = seconds / _breatheSeconds % 1;
      final dim = 1 - 0.55 * (1 - (phase * 2 - 1).abs());
      track(dim);
    case ActivityKind.optimize:
      track(0.18);
      canvas.drawArc(
        rect,
        top,
        turn * ring.progress,
        false,
        stroke..color = tone,
      );
    case ActivityKind.warn || ActivityKind.bad:
      track(1);
  }
}
