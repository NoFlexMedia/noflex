// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:jovial_svg/jovial_svg.dart';

/// The colour of the logo at a point in its own coordinates. These logos set
/// their colours from a `<style>` block; a renderer that ignores it draws them
/// solid black.
Future<Color> _pixel(String path, double x, double y) async {
  final logo = ScalableImage.fromSvgString(File(path).readAsStringSync());
  final recorder = ui.PictureRecorder();
  logo.paint(Canvas(recorder));
  final size = logo.viewport;
  final image = await recorder.endRecording().toImage(
    size.right.ceil(),
    size.bottom.ceil(),
  );
  final bytes = await image.toByteData();
  final at = (y.round() * image.width + x.round()) * 4;
  return Color.fromARGB(
    bytes!.getUint8(at + 3),
    bytes.getUint8(at),
    bytes.getUint8(at + 1),
    bytes.getUint8(at + 2),
  );
}

void main() {
  const plugins = '../backend/plugins';

  testWidgets('TMDB keeps the gradient its style block gives it', (
    tester,
  ) async {
    await tester.runAsync(() async {
      final pill = await _pixel(
        '$plugins/noflex-metadata-tmdb/assets/tmdb-logo.svg',
        170,
        18,
      );
      expect((pill.a * 255).round(), 255);
      expect(pill.b, greaterThan(0.7), reason: 'the blue end, got $pill');
    });
  });

  testWidgets('MusicBrainz keeps both of its colours', (tester) async {
    await tester.runAsync(() async {
      const logo =
          '$plugins/noflex-metadata-musicbrainz/assets/musicbrainz-logo.svg';
      final left = await _pixel(logo, 7, 15);
      final right = await _pixel(logo, 20, 15);
      expect(left, const Color(0xFFBA478F));
      expect(right, const Color(0xFFEB743B));
    });
  });
}
