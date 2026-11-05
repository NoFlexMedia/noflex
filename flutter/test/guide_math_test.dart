// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/livetv/guide_math.dart';

GuideProgram program(
  String id,
  DateTime start,
  DateTime end, {
  String channel = 'c',
}) =>
    GuideProgram(id: id, channelId: channel, title: id, start: start, end: end);

DateTime utc(int hour, [int minute = 0]) =>
    DateTime.utc(2026, 11, 5, hour, minute);

void main() {
  group('guideAnchor', () {
    test('starts the half hour before the current one', () {
      final anchor = guideAnchor(utc(10, 47), utcOffset: Duration.zero);
      expect(anchor.toUtc(), utc(10));
    });

    test('lands on the half hour exactly when now does', () {
      final anchor = guideAnchor(utc(10, 30), utcOffset: Duration.zero);
      expect(anchor.toUtc(), utc(10));
    });

    test('follows local half hours in a zone offset by 45 minutes', () {
      // 10:47 UTC is 16:32 in Nepal, so the lines belong at 16:00 local.
      const nepal = Duration(hours: 5, minutes: 45);
      final anchor = guideAnchor(utc(10, 47), utcOffset: nepal);
      expect(anchor.toUtc(), utc(10, 15));
    });
  });

  test('the window only moves once less than three hours of it remain', () {
    final anchor = utc(10);
    expect(guideWindowIsStale(anchor, utc(18, 59)), isFalse);
    expect(guideWindowIsStale(anchor, utc(19)), isFalse);
    expect(guideWindowIsStale(anchor, utc(19, 1)), isTrue);
  });

  group('guideRows', () {
    final start = utc(10);
    final end = utc(22);

    test('clips a program running into the window from before it', () {
      final rows = guideRows(
        [program('a', utc(9), utc(11))],
        start: start,
        end: end,
      );
      expect(rows['c']!.single.startMinutes, 0);
      expect(rows['c']!.single.lengthMinutes, 60);
    });

    test('clips a program running past the end of the window', () {
      final rows = guideRows(
        [program('a', utc(21), utc(23, 30))],
        start: start,
        end: end,
      );
      expect(rows['c']!.single.startMinutes, 660);
      expect(rows['c']!.single.lengthMinutes, 60);
    });

    test('leaves out what only touches the window', () {
      final rows = guideRows(
        [
          program('before', utc(9), utc(10)),
          program('after', utc(22), utc(23)),
        ],
        start: start,
        end: end,
      );
      expect(rows, isEmpty);
    });

    test('keeps channels apart and each in order', () {
      final rows = guideRows(
        [
          program('later', utc(12), utc(13)),
          program('elsewhere', utc(11), utc(12), channel: 'other'),
          program('earlier', utc(10), utc(12)),
        ],
        start: start,
        end: end,
      );
      expect(rows['c']!.map((b) => b.program.id), ['earlier', 'later']);
      expect(rows['other']!.single.program.id, 'elsewhere');
    });
  });

  test('now is only placed while it falls inside the window', () {
    expect(guideNowMinutes(utc(10), utc(22), utc(10, 45)), 45);
    expect(guideNowMinutes(utc(10), utc(22), utc(9, 59)), isNull);
    expect(guideNowMinutes(utc(10), utc(22), utc(22, 1)), isNull);
  });

  group('nowAndNext', () {
    final programs = [
      program('past', utc(9), utc(10)),
      program('later', utc(12), utc(13)),
      program('current', utc(10), utc(11)),
      program('next', utc(11), utc(12)),
    ];

    test('picks what’s on and the earliest thing after it', () {
      final (:current, :next) = nowAndNext(programs, utc(10, 30));
      expect(current?.id, 'current');
      expect(next?.id, 'next');
    });

    test('a program ending now has given way to the one starting now', () {
      final (:current, :next) = nowAndNext(programs, utc(11));
      expect(current?.id, 'next');
      expect(next?.id, 'later');
    });

    test('a gap in the schedule has nothing on but something next', () {
      final (:current, :next) = nowAndNext([
        program('later', utc(12), utc(13)),
      ], utc(11));
      expect(current, isNull);
      expect(next?.id, 'later');
    });
  });

  test('channels sort by number, including subchannels, then by name', () {
    final channels = [
      (null, 'zebra'),
      ('10', 'ten'),
      ('5.2', 'b'),
      ('5.1', 'a'),
      (null, 'Apple'),
      ('abc', 'mango'),
    ]..sort((a, b) => compareChannels(a.$1, a.$2, b.$1, b.$2));
    expect(channels.map((c) => c.$2), [
      'a',
      'b',
      'ten',
      'Apple',
      'mango',
      'zebra',
    ]);
  });
}
