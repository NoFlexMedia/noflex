// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/cast/cast_discovery.dart';
import 'package:noflex/features/cast/cast_message.dart';

void main() {
  const message = CastMessage(
    source: 'a',
    destination: 'b',
    namespace: 'c',
    payload: '{}',
  );

  test('a message is the protobuf Cast devices read, behind its length', () {
    // CastMessage's fields by number: version 0, three strings, STRING, the payload.
    expect(message.encode(), [
      0, 0, 0, 17, //
      0x08, 0, //
      0x12, 1, 0x61, //
      0x1a, 1, 0x62, //
      0x22, 1, 0x63, //
      0x28, 0, //
      0x32, 2, 0x7b, 0x7d,
    ]);
  });

  test('messages come back whole however the stream splits them', () {
    final long = CastMessage(
      source: 'receiver-0',
      destination: '*',
      namespace: 'urn:x-cast:com.google.cast.media',
      // Past 127 bytes, so lengths take two varint bytes.
      payload: '{"type":"MEDIA_STATUS","pad":"${'x' * 300}"}',
    );
    final bytes = [...message.encode(), ...long.encode()];
    final reader = CastFrameReader();
    final received = <CastMessage>[];
    for (var at = 0; at < bytes.length; at += 7) {
      received.addAll(
        reader.add(bytes.sublist(at, (at + 7).clamp(0, bytes.length))),
      );
    }
    expect(received.map((m) => m.payload), [message.payload, long.payload]);
    expect(received.last.namespace, long.namespace);
    expect(received.last.destination, '*');
  });

  test('fields a sender ignores are skipped', () {
    // A binary payload (field 7) and an unknown 32-bit field before the payload.
    final body = Uint8List.fromList([
      0x08, 0, 0x12, 1, 0x61, 0x1a, 1, 0x62, 0x22, 1, 0x63, //
      0x3a, 2, 0xff, 0xfe, //
      0x45, 1, 2, 3, 4, //
      0x32, 2, 0x7b, 0x7d,
    ]);
    final decoded = CastMessage.decode(body);
    expect(decoded.payload, '{}');
    expect(decoded.source, 'a');
  });

  test('speakers and speaker groups are left out, screens kept', () {
    // `ca` as a Nest Hub, a Pixel Tablet, a JBL bar and a speaker group announce it.
    expect(showsVideo({'ca': '231941'}), isTrue);
    expect(showsVideo({'ca': '231429'}), isTrue);
    expect(showsVideo({'ca': '199172'}), isFalse);
    expect(showsVideo({'ca': '199204'}), isFalse);
  });
}
