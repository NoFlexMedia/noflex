// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/player/playback_position.dart';

const film = 1801000;

bool falseEnd({
  required int at,
  required int reached,
  required int requested,
}) => isFalseEnd(
  streamMs: at,
  streamLengthMs: film,
  atMs: at,
  reachedMs: reached,
  requestedMs: requested,
);

void main() {
  group('isFalseEnd', () {
    test('libmpv jumping to the last frame after a failed seek', () {
      expect(falseEnd(at: film, reached: 300000, requested: 300000), isTrue);
    });

    test('playing through the last frames', () {
      expect(falseEnd(at: film, reached: film - 500, requested: 0), isFalse);
    });

    test('the viewer seeking to the very end', () {
      expect(falseEnd(at: film, reached: 300000, requested: film), isFalse);
    });

    test('a jump that stops short of the end', () {
      expect(falseEnd(at: 900000, reached: 300000, requested: 300000), isFalse);
    });

    test('a stream whose length is not known yet', () {
      expect(
        isFalseEnd(
          streamMs: 900000,
          streamLengthMs: 0,
          atMs: 900000,
          reachedMs: 300000,
          requestedMs: 300000,
        ),
        isFalse,
      );
    });
  });

  group('endedEarly', () {
    test('a stream lost mid-film', () {
      expect(endedEarly(lengthMs: film, reachedMs: 608000), isTrue);
    });

    test('a file a little shorter than its probed length', () {
      expect(endedEarly(lengthMs: film, reachedMs: film - 20000), isFalse);
    });

    test('a title of unknown length', () {
      expect(endedEarly(lengthMs: 0, reachedMs: 608000), isFalse);
    });
  });
}
