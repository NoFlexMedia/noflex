// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../di.dart';
import '../services/tokens.dart';
import '../theme.dart';
import '../utils/ambient.dart';

/// A page's tint, drawn from the artwork at [url] and easing to the next. Goes
/// behind a page's content: the web tints beneath every page, but here each
/// route is opaque, so a tint under them all would never be seen.
class AmbientGlow extends StatefulWidget {
  const AmbientGlow({super.key, required this.url});

  final String? url;

  @override
  State<AmbientGlow> createState() => _AmbientGlowState();
}

class _AmbientGlowState extends State<AmbientGlow> {
  final _color = signal(NfColors.ambient);

  @override
  void initState() {
    super.initState();
    _sample();
  }

  @override
  void didUpdateWidget(AmbientGlow oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.url != widget.url) _sample();
  }

  Future<void> _sample() async {
    final url = widget.url;
    final color = url == null
        ? null
        : await ambientColor(getIt<TokenStore>().absolute(url));
    if (!mounted || widget.url != url) return;
    _color.value = color ?? NfColors.ambient;
  }

  @override
  Widget build(BuildContext context) {
    final still = MediaQuery.disableAnimationsOf(context);
    return IgnorePointer(
      child: FractionallySizedBox(
        alignment: Alignment.topCenter,
        heightFactor: 0.9,
        child: SignalBuilder(
          builder: (_) => TweenAnimationBuilder<Color?>(
            tween: ColorTween(end: _color.value),
            duration: still
                ? Duration.zero
                : const Duration(milliseconds: 1200),
            curve: NfMotion.easeOut,
            builder: (_, color, _) => CustomPaint(
              painter: _GlowPainter(color ?? NfColors.ambient),
              size: Size.infinite,
            ),
          ),
        ),
      ),
    );
  }
}

/// `radial-gradient(120% 70% at 60% -10%, ...)`, the web's mask over its tint.
class _GlowPainter extends CustomPainter {
  const _GlowPainter(this.color);

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final ellipse = Float64List.fromList([
      size.width * 1.2, 0, 0, 0, //
      0, size.height * 0.7, 0, 0, //
      0, 0, 1, 0, //
      size.width * 0.6, size.height * -0.1, 0, 1, //
    ]);
    canvas.drawRect(
      Offset.zero & size,
      Paint()
        ..shader = ui.Gradient.radial(
          Offset.zero,
          1,
          [color.withValues(alpha: 0.34), color.withValues(alpha: 0)],
          const [0, 0.7],
          TileMode.clamp,
          ellipse,
        ),
    );
  }

  @override
  bool shouldRepaint(_GlowPainter oldDelegate) => oldDelegate.color != color;
}
