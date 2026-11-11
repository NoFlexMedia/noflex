// SPDX-License-Identifier: AGPL-3.0-only
//
// Cast's wire format (CASTV2): each message is a protobuf `CastMessage` behind
// a four-byte big-endian length. It has seven fields, so it's encoded here by
// hand rather than through a protobuf dependency, and only string payloads are
// sent: every namespace a sender needs speaks JSON.

import 'dart:convert';
import 'dart:typed_data';

class CastMessage {
  const CastMessage({
    required this.source,
    required this.destination,
    required this.namespace,
    required this.payload,
  });

  final String source;
  final String destination;
  final String namespace;

  /// JSON text, or empty for a binary payload (device authentication's).
  final String payload;

  /// The framed bytes: length, then the protobuf.
  Uint8List encode() {
    final body = BytesBuilder(copy: false)
      ..add(_varintField(1, 0)) // protocol_version: CASTV2_1_0
      ..add(_stringField(2, source))
      ..add(_stringField(3, destination))
      ..add(_stringField(4, namespace))
      ..add(_varintField(5, 0)) // payload_type: STRING
      ..add(_stringField(6, payload));
    final bytes = body.takeBytes();
    return (BytesBuilder(copy: false)
          ..add((ByteData(4)..setUint32(0, bytes.length)).buffer.asUint8List())
          ..add(bytes))
        .takeBytes();
  }

  /// One protobuf `CastMessage`, without its length.
  static CastMessage decode(Uint8List bytes) {
    final strings = <int, String>{};
    var at = 0;
    int varint() {
      var value = 0;
      for (var shift = 0; ; shift += 7) {
        final byte = bytes[at++];
        value |= (byte & 0x7f) << shift;
        if (byte < 0x80) return value;
      }
    }

    while (at < bytes.length) {
      final key = varint();
      switch (key & 7) {
        case 0:
          varint();
        case 1:
          at += 8;
        case 2:
          final length = varint();
          final field = key >> 3;
          if (field != 7) {
            strings[field] = utf8.decode(
              Uint8List.sublistView(bytes, at, at + length),
            );
          }
          at += length;
        case 5:
          at += 4;
        default:
          throw const FormatException('Not a Cast message');
      }
    }
    return CastMessage(
      source: strings[2] ?? '',
      destination: strings[3] ?? '',
      namespace: strings[4] ?? '',
      payload: strings[6] ?? '',
    );
  }

  static List<int> _varint(int value) {
    final out = <int>[];
    var rest = value;
    while (rest >= 0x80) {
      out.add((rest & 0x7f) | 0x80);
      rest >>= 7;
    }
    return out..add(rest);
  }

  static List<int> _varintField(int field, int value) => [
    ..._varint(field << 3),
    ..._varint(value),
  ];

  static List<int> _stringField(int field, String value) {
    final bytes = utf8.encode(value);
    return [..._varint(field << 3 | 2), ..._varint(bytes.length), ...bytes];
  }
}

/// Cuts a TLS byte stream back into messages, however it arrives in chunks.
class CastFrameReader {
  final _pending = BytesBuilder(copy: false);

  List<CastMessage> add(List<int> chunk) {
    _pending.add(chunk);
    final bytes = _pending.takeBytes();
    final messages = <CastMessage>[];
    var at = 0;
    while (bytes.length - at >= 4) {
      final length = ByteData.sublistView(bytes, at, at + 4).getUint32(0);
      if (bytes.length - at - 4 < length) break;
      messages.add(
        CastMessage.decode(
          Uint8List.sublistView(bytes, at + 4, at + 4 + length),
        ),
      );
      at += 4 + length;
    }
    if (at < bytes.length) _pending.add(Uint8List.sublistView(bytes, at));
    return messages;
  }
}
