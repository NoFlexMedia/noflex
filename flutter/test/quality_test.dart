// SPDX-License-Identifier: AGPL-3.0-only
//
// Mirrors solidtv/src/lib/quality.test.ts, which tests the policy this ports.

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/player/quality.dart';

void main() {
  late int now;
  late AutoQuality auto;
  setUp(() {
    now = 0;
    auto = AutoQuality(() => now);
  });
  void at(int ms) => now = ms;

  group('AutoQuality', () {
    test('leaves the original after repeated stalls, but not for filling the '
        'buffer', () {
      auto.settle();
      at(1000);
      auto.stalled();
      at(1500);
      auto.resumed();
      expect(auto.struggling(), isFalse);
      at(20000);
      auto.stalled();
      at(21000);
      auto.resumed();
      expect(auto.struggling(), isFalse);
      at(40000);
      auto.stalled();
      expect(auto.struggling(), isTrue);
    });

    test('forgets stalls that fell out of the window', () {
      at(10000);
      auto.stalled();
      auto.resumed();
      at(10000 + Auto.windowMs + 1);
      auto.stalled();
      auto.resumed();
      expect(auto.struggling(), isFalse);
    });

    test('takes one long stall as enough', () {
      at(10000);
      auto.stalled();
      at(10000 + Auto.longStallMs - 1);
      expect(auto.struggling(), isFalse);
      at(10000 + Auto.longStallMs);
      expect(auto.struggling(), isTrue);
    });

    test('goes back to the original only after holding, with headroom the '
        'whole time', () {
      const original = 10000000;
      expect(auto.recovered(50000000, original), isFalse);
      auto.leftOriginal();
      // Comfortable for long enough, but too soon after leaving.
      at(1000);
      expect(auto.recovered(20000000, original), isFalse);
      at(1000 + Auto.recoverMs);
      expect(auto.recovered(20000000, original), isFalse);
      // A dip below the headroom starts the clock again.
      at(Auto.holdMs - 10000);
      expect(auto.recovered(12000000, original), isFalse);
      at(Auto.holdMs);
      expect(auto.recovered(20000000, original), isFalse);
      at(Auto.holdMs + Auto.recoverMs - 1);
      expect(auto.recovered(20000000, original), isFalse);
      at(Auto.holdMs + Auto.recoverMs);
      expect(auto.recovered(20000000, original), isTrue);
    });
  });

  group('sessionQuality', () {
    test('asks for the ladder under the original once Auto has left it', () {
      expect(sessionQuality(QualityChoice.auto, 20000000), (
        adaptive: true,
        maxBitrateBps: null,
      ));
      expect(sessionQuality(QualityChoice.auto, 20000000, leftOriginal: true), (
        adaptive: true,
        maxBitrateBps: 18000000,
      ));
      expect(sessionQuality(const QualityChoice.pinned(4000000), null), (
        adaptive: false,
        maxBitrateBps: 4000000,
      ));
    });

    test('keeps a choice as text', () {
      for (final choice in const [
        QualityChoice.original,
        QualityChoice.auto,
        QualityChoice.pinned(8000000),
      ]) {
        expect(decodeChoice(encodeChoice(choice)), choice);
      }
      expect(decodeChoice('nonsense'), QualityChoice.original);
    });
  });

  group('frameLabel', () {
    for (final (width, height, label) in const [
      (1280, 544, '720p'),
      (1920, 1080, '1080p'),
      (1920, 1088, '1080p'),
      (3840, 1600, '4K'),
      (1440, 1080, '1080p'),
      (720, 576, '720p'),
    ]) {
      test('names ${width}x$height $label', () {
        expect(frameLabel(width, height), label);
      });
    }
  });
}
