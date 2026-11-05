// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/player/capabilities.dart';
import 'package:noflex/features/player/decoders.dart';
import 'package:noflex/graphql/__generated__/schema.schema.gql.dart';
import 'package:noflex/models/media_probe.dart';

MediaVersion version(
  String id, {
  required String codec,
  required int height,
  int depth = 8,
  int part = 0,
  int size = 1000,
  String? transfer,
  String edition = 'e',
}) => MediaVersion(
  id: id,
  editionId: edition,
  partIndex: part,
  sizeBytes: size,
  probe: MediaProbe(
    video: VideoProbe(
      codec: codec,
      width: height * 16 ~/ 9,
      height: height,
      bitDepth: depth,
      transfer: transfer,
    ),
  ),
);

/// A phone: hardware H.264 and 8-bit HEVC, no AV1.
const phone = DecoderProfile(
  hardware: {
    'h264': [DecodeCeiling(pixels2160p, 8)],
    'h265': [DecodeCeiling(pixels1080p, 8)],
  },
  softwareCeiling: DecodeCeiling(pixels1080p, 10),
);

/// A desktop with a recent GPU: everything in hardware, 4K 10-bit included.
const desktop = DecoderProfile(
  hardware: {
    'h264': [DecodeCeiling(pixels2160p, 8)],
    'h265': [DecodeCeiling(pixels2160p, 10)],
    'av1': [DecodeCeiling(pixels2160p, 10)],
  },
  softwareCeiling: DecodeCeiling(pixels2160p, 10),
);

void main() {
  group('rankVersions', () {
    final library = [
      version('av1-1080', codec: 'av1', height: 1080, size: 36),
      version('h264-1080', codec: 'h264', height: 1080, size: 40),
      version('hevc10-2160', codec: 'h265', height: 2160, depth: 10, size: 46),
      version('av110-2160', codec: 'av1', height: 2160, depth: 10, size: 32),
    ];

    test('a smaller hardware version beats a bigger picture in software', () {
      final ranked = rankVersions(library, phone, displayPixels: pixels2160p);
      expect(ranked.first.id, 'h264-1080');
      // The 1080p AV1 at least plays in software; the 4K ones need the server.
      expect(ranked[1].id, 'av1-1080');
    });

    test(
      'takes the bigger picture when hardware can and the display shows it',
      () {
        final ranked = rankVersions(
          library,
          desktop,
          displayPixels: pixels2160p,
        );
        expect(ranked.first.probe.video!.height, 2160);
      },
    );

    test('doesn’t spend decode power on pixels the display can’t show', () {
      final ranked = rankVersions(library, desktop, displayPixels: pixels1080p);
      expect(ranked.first.probe.video!.height, 1080);
    });

    test('an SDR master beats HDR the player would have to tone-map', () {
      final ranked = rankVersions(
        [
          version(
            'hdr',
            codec: 'h265',
            height: 2160,
            depth: 10,
            transfer: 'pq',
          ),
          version('sdr', codec: 'h265', height: 2160, depth: 10),
        ],
        desktop,
        displayPixels: pixels2160p,
      );
      expect(ranked.first.id, 'sdr');
    });
  });

  group('pickEdition', () {
    test('prefers the first edition with a hardware version, over order', () {
      final choice = pickEdition(
        [
          ('theatrical', [version('a', codec: 'av1', height: 1080)]),
          ('directors', [version('b', codec: 'h264', height: 1080)]),
        ],
        phone,
        displayPixels: pixels1080p,
      );
      expect(choice!.editionId, 'directors');
      expect(choice.tier, DecodeTier.hardware);
    });

    test(
      'keeps the server’s order when editions tie, whatever their picture',
      () {
        final choice = pickEdition(
          [
            ('theatrical', [version('a', codec: 'h264', height: 720)]),
            ('directors', [version('b', codec: 'h264', height: 2160)]),
          ],
          phone,
          displayPixels: pixels2160p,
        );
        expect(choice!.editionId, 'theatrical');
      },
    );

    test('judges a multi-part edition by its first part', () {
      final choice = pickEdition(
        [
          (
            'long',
            [
              version('part2', codec: 'av1', height: 1080, part: 1),
              version('part1', codec: 'h264', height: 1080, part: 0),
            ],
          ),
        ],
        phone,
        displayPixels: pixels1080p,
      );
      expect(choice!.best!.id, 'part1');
    });
  });

  test('a file that fell back to software loses its hardware ceiling', () {
    final video = version('a', codec: 'h264', height: 2160).probe.video!;
    expect(phone.tierOf(video), DecodeTier.hardware);
    final corrected = phone.withoutHardware(video);
    expect(corrected.tierOf(video), isNot(DecodeTier.hardware));
    // Other codecs are unaffected.
    expect(
      corrected.tierOf(version('b', codec: 'h265', height: 1080).probe.video!),
      DecodeTier.hardware,
    );
  });

  group('buildCapabilities', () {
    GVideoCodecSupport support(GClientCapabilities caps, GVideoCodec codec) =>
        caps.videoCodecs.firstWhere((c) => c.codec == codec);

    test('reports the better of hardware and software per codec', () {
      final caps = buildCapabilities(phone);
      // Hardware H.264 reaches 4K; AV1 only has the software ceiling.
      expect(support(caps, GVideoCodec.H264).maxPixels, pixels2160p);
      expect(support(caps, GVideoCodec.AV1).maxPixels, pixels1080p);
    });

    test('leaves out Dolby Vision without a compatible base layer', () {
      final ranges = support(
        buildCapabilities(desktop),
        GVideoCodec.H265,
      ).videoRanges;
      expect(ranges, contains(GVideoRange.HDR10));
      expect(ranges, contains(GVideoRange.DOVI_WITH_HDR10));
      expect(ranges, isNot(contains(GVideoRange.DOVI)));
    });

    test('only claims audio this build of libmpv can decode', () {
      const windows = DecoderProfile(
        softwareCeiling: DecodeCeiling(pixels1080p, 10),
        audioDecoders: {'aac', 'ac3', 'eac3', 'dts'},
      );
      final caps = buildCapabilities(windows);
      final direct = caps.directPlay.firstWhere((p) => p.container == 'mkv');
      expect(direct.audioCodecs, contains(GAudioCodec.DTS));
      expect(direct.audioCodecs, isNot(contains(GAudioCodec.TRUEHD)));
    });
  });
}
