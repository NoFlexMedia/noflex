// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/account/account_controller.dart';
import 'package:noflex/features/player/decoders.dart';
import 'package:noflex/utils/tracks.dart';

void main() {
  group('mergePlaybackPreference', () {
    const prefs = PlaybackPrefs(
      audioLanguages: ['fr'],
      subtitlesOn: true,
      subtitleLanguages: ['en', 'de'],
    );

    test('keeps what other clients stored beside these choices', () {
      final merged = mergePlaybackPreference({
        'quality': '1080',
        'autoplayNext': false,
        'audio': {
          'languages': ['ja'],
          'normalize': true,
        },
        'subtitles': {'enabled': false, 'size': 'large'},
      }, prefs);
      expect(merged['quality'], '1080');
      expect(merged['autoplayNext'], false);
      expect(merged['audio'], {
        'languages': ['fr'],
        'normalize': true,
      });
      expect(merged['subtitles'], {
        'enabled': true,
        'languages': ['en', 'de'],
        'size': 'large',
      });
    });

    test('starts clean from nothing, or from a shape it can’t use', () {
      for (final stored in [null, 'corrupt', 3]) {
        expect(mergePlaybackPreference(stored, prefs), prefs.toJson());
      }
      final merged = mergePlaybackPreference({'audio': 'corrupt'}, prefs);
      expect(merged['audio'], {
        'languages': ['fr'],
      });
    });

    test('round-trips through what playback reads', () {
      final read = PlaybackPrefs.fromJson(
        jsonDecode(
          jsonEncode(mergePlaybackPreference({'quality': 'auto'}, prefs)),
        ),
      );
      expect(read.audioLanguages, prefs.audioLanguages);
      expect(read.subtitlesOn, isTrue);
      expect(read.subtitleLanguages, prefs.subtitleLanguages);
    });
  });

  group('sessionIdOfToken', () {
    String token(Map<String, Object> claims) =>
        'header.${base64Url.encode(utf8.encode(jsonEncode(claims))).replaceAll('=', '')}.sig';

    test('reads the sid claim from an unpadded payload', () {
      expect(sessionIdOfToken(token({'sid': 'abc', 'sub': 'u'})), 'abc');
    });

    test('anything unreadable is no session rather than a failure', () {
      expect(sessionIdOfToken(null), isNull);
      expect(sessionIdOfToken('not-a-jwt'), isNull);
      expect(sessionIdOfToken('a.%%%.c'), isNull);
      expect(sessionIdOfToken(token({'sub': 'u'})), isNull);
    });
  });

  group('decoderSummary', () {
    DecoderProfile profile(List<DecodeCeiling> ceilings) => DecoderProfile(
      hardware: {'h265': ceilings},
      softwareCeiling: const DecodeCeiling(pixels1080p, 10),
    );

    test('says where 10-bit tops out below 8-bit', () {
      final summary = decoderSummary(
        'h265',
        profile(const [
          DecodeCeiling(pixels1080p, 8),
          DecodeCeiling(pixels2160p, 8),
          DecodeCeiling(pixels1080p, 10),
        ]),
      );
      expect(summary.hardware, isTrue);
      expect(summary.detail, 'Hardware, 4K 8-bit, 1080p 10-bit');
    });

    test('one line when 10-bit reaches as far as 8-bit', () {
      final summary = decoderSummary(
        'h265',
        profile(const [
          DecodeCeiling(pixels2160p, 8),
          DecodeCeiling(pixels2160p, 10),
        ]),
      );
      expect(summary.detail, 'Hardware, 4K, 10-bit');
    });

    test('a codec with nothing measured falls back to software', () {
      final none = decoderSummary('av1', profile(const []));
      expect(none.hardware, isFalse);
      expect(none.detail, 'Software, up to 1080p');
      // What's left after real playback disproved every measurement.
      expect(decoderSummary('h265', profile(const [])).hardware, isFalse);
    });
  });
}
