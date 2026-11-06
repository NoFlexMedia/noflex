// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/utils/picture.dart';
import 'package:noflex/widgets/user_avatar.dart';

Future<Uint8List> _png(int width, int height) async {
  final recorder = ui.PictureRecorder();
  ui.Canvas(recorder).drawColor(const ui.Color(0xFF3366CC), ui.BlendMode.src);
  final image = await recorder.endRecording().toImage(width, height);
  final data = await image.toByteData(format: ui.ImageByteFormat.png);
  return data!.buffer.asUint8List();
}

Future<(int, int)> _size(String encoded) async {
  final codec = await ui.instantiateImageCodec(base64Decode(encoded));
  final image = (await codec.getNextFrame()).image;
  return (image.width, image.height);
}

void main() {
  test('initials are the first and last words, or a lone word’s start', () {
    expect(initialsOf('Vera Viewer'), 'VV');
    expect(initialsOf('Vera van der Viewer'), 'VV');
    expect(initialsOf('vera'), 'VE');
    expect(initialsOf('  '), '');
    expect(initialsOf(null), '');
  });

  testWidgets('a picture goes up as a small square', (tester) async {
    await tester.runAsync(() async {
      expect(await _size(await pictureForUpload(await _png(3000, 1500))), (
        1024,
        1024,
      ));
      expect(await _size(await pictureForUpload(await _png(200, 400))), (
        200,
        200,
      ));
    });
  });
}
