// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/player/ladder.dart';
import 'package:noflex/features/player/quality.dart';

// As the server writes an adaptive session's master, abridged.
const _master = '''
#EXTM3U
#EXT-X-VERSION:7
#EXT-X-INDEPENDENT-SEGMENTS
#EXT-X-STREAM-INF:BANDWIDTH=1252000,RESOLUTION=640x360,CODECS="avc1.64001F,mp4a.40.2",VIDEO-RANGE=SDR
v2.m3u8?token=t
#EXT-X-STREAM-INF:BANDWIDTH=4300000,RESOLUTION=1280x720,CODECS="avc1.64001F,mp4a.40.2",VIDEO-RANGE=SDR
v0.m3u8?token=t
#EXT-X-STREAM-INF:BANDWIDTH=2123000,RESOLUTION=852x480,CODECS="avc1.64001F,mp4a.40.2",VIDEO-RANGE=SDR
v1.m3u8?token=t
''';

void main() {
  final variants = parseMaster(
    _master,
    Uri.parse('http://server/stream/job/1/master.m3u8?token=t'),
  );

  test('reads the variants of a master, best first', () {
    expect([for (final v in variants) v.height], [720, 480, 360]);
    expect(
      variants.first.uri.toString(),
      'http://server/stream/job/1/v0.m3u8?token=t',
    );
    expect(
      parseMaster('#EXTM3U\n#EXTINF:4.0,\nv0_00000.m4s\n', Uri()),
      isEmpty,
    );
  });

  test('starts on the best variant the estimate holds', () {
    expect(LadderSteps.pick(variants, null), 0);
    expect(LadderSteps.pick(variants, 3000000), 1);
    expect(LadderSteps.pick(variants, 500000), 2);
  });

  group('LadderSteps', () {
    late int now;
    late LadderSteps steps;
    setUp(() {
      now = 0;
      steps = LadderSteps(variants, 0, now: () => now);
    });

    test('steps down on stalls, as far as the estimate says', () {
      now = 10000;
      steps.stalled();
      now = 10000 + Auto.longStallMs;
      expect(steps.next(20000000), 1);
      steps.resumed();
      // The move settles, so filling the new stream's buffer isn't a stall.
      steps.stalled();
      now += Auto.longStallMs;
      expect(steps.next(20000000), isNull);
      steps.resumed();
      now += Auto.settleMs;
      steps.stalled();
      now += Auto.longStallMs;
      expect(steps.next(600000), 2);
      steps.resumed();
      now += Auto.settleMs;
      steps.stalled();
      now += Auto.longStallMs;
      expect(steps.next(600000), isNull, reason: 'nowhere lower to go');
    });

    test('steps up once a better variant has fit for a while', () {
      steps = LadderSteps(variants, 2, now: () => now);
      now = Steps.stayMs - Steps.upMs;
      expect(steps.next(10000000), isNull);
      now = Steps.stayMs - 1;
      expect(steps.next(10000000), isNull);
      // A dip starts the clock again.
      expect(steps.next(1000000), isNull);
      now = Steps.stayMs;
      expect(steps.next(10000000), isNull);
      now = Steps.stayMs + Steps.upMs;
      expect(steps.next(10000000), 0);
      expect(steps.at, 0);
    });
  });
}
