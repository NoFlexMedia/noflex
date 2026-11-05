// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';

import '../theme.dart';

/// The mark from web/static/favicon.svg.
class BrandMark extends StatelessWidget {
  const BrandMark({super.key, this.size = 30});
  final double size;

  @override
  Widget build(BuildContext context) =>
      CustomPaint(size: Size.square(size), painter: const _BrandPainter());
}

class _BrandPainter extends CustomPainter {
  const _BrandPainter();

  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 64);
    final squircle = Path()
      ..moveTo(32, 2)
      ..cubicTo(61.6, 2, 62, 2.4, 62, 32)
      ..cubicTo(62, 61.6, 61.6, 62, 32, 62)
      ..cubicTo(2.4, 62, 2, 61.6, 2, 32)
      ..cubicTo(2, 2.4, 2.4, 2, 32, 2)
      ..close();
    canvas.drawPath(squircle, Paint()..color = NfColors.accent);
    final ink = Paint()..color = NfColors.accentInk;
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        const Rect.fromLTWH(13.5, 14, 6.5, 36),
        const Radius.circular(3.25),
      ),
      ink,
    );
    final crown = Path()
      ..moveTo(23.5, 15)
      ..lineTo(37, 20)
      ..lineTo(30.5, 26.5)
      ..lineTo(50, 32)
      ..lineTo(30.5, 37.5)
      ..lineTo(37, 44)
      ..lineTo(23.5, 49)
      ..close();
    canvas.drawPath(crown, ink);
    canvas.drawPath(
      crown,
      Paint()
        ..color = NfColors.accentInk
        ..style = PaintingStyle.stroke
        ..strokeWidth = 3
        ..strokeJoin = StrokeJoin.round,
    );
  }

  @override
  bool shouldRepaint(_BrandPainter oldDelegate) => false;
}
