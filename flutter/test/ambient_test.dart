// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/utils/ambient.dart';

Uint8List _pixels(List<(int, int, int)> colors) => Uint8List.fromList([
  for (final (r, g, b) in colors) ...[r, g, b, 255],
]);

void main() {
  test('a little saturated color outweighs a lot of black and white', () {
    final color = ambientFrom(
      _pixels([
        for (var i = 0; i < 12; i++) (5, 5, 5),
        for (var i = 0; i < 12; i++) (255, 255, 255),
        for (var i = 0; i < 3; i++) (40, 90, 200),
      ]),
    )!;
    expect(color.b, greaterThan(color.r));
    expect(color.b, greaterThan(color.g));
  });

  test('stays bright enough to see and dim enough to read over', () {
    final black = ambientFrom(_pixels([(0, 0, 0)]))!;
    final white = ambientFrom(_pixels([(255, 255, 255)]))!;
    expect((black.r * 255).round(), 40);
    expect((white.r * 255).round(), 200);
  });

  test('no pixels give no tint', () {
    expect(ambientFrom(Uint8List(0)), isNull);
  });
}
