// SPDX-License-Identifier: AGPL-3.0-only
//
// Profile pictures. This sends a square small enough not to matter on a phone
// connection, or the file as it is when the platform can't read it (AVIF, on most),
// which the server can. The server re-encodes it either way.

import 'dart:convert';
import 'dart:typed_data';
import 'dart:ui' as ui;

const _side = 1024;

/// `bytes` as the base64 `setProfilePicture` takes.
Future<String> pictureForUpload(Uint8List bytes) async {
  try {
    return await _square(bytes);
  } catch (_) {
    return base64Encode(bytes);
  }
}

/// A centred square of the picture in `bytes`. Throws if the platform can't read it.
Future<String> _square(Uint8List bytes) async {
  final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
  final descriptor = await ui.ImageDescriptor.encoded(buffer);
  final short = descriptor.width < descriptor.height
      ? descriptor.width
      : descriptor.height;
  final scale = short > _side ? _side / short : 1.0;
  final codec = await descriptor.instantiateCodec(
    targetWidth: (descriptor.width * scale).round(),
    targetHeight: (descriptor.height * scale).round(),
  );
  final image = (await codec.getNextFrame()).image;
  final side = image.width < image.height ? image.width : image.height;
  final recorder = ui.PictureRecorder();
  ui.Canvas(recorder).drawImageRect(
    image,
    ui.Rect.fromCenter(
      center: ui.Offset(image.width / 2, image.height / 2),
      width: side.toDouble(),
      height: side.toDouble(),
    ),
    ui.Rect.fromLTWH(0, 0, side.toDouble(), side.toDouble()),
    ui.Paint(),
  );
  final square = await recorder.endRecording().toImage(side, side);
  final encoded = await square.toByteData(format: ui.ImageByteFormat.png);
  image.dispose();
  square.dispose();
  codec.dispose();
  descriptor.dispose();
  buffer.dispose();
  return base64Encode(encoded!.buffer.asUint8List());
}
