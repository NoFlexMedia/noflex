// SPDX-License-Identifier: AGPL-3.0-only
//
// The page tint sampled from artwork. The web's `ambientColor`, in lib/images.ts.

import 'dart:async';
import 'dart:math';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter/painting.dart';

import 'images.dart';

final _sampled = <String, Color>{};

/// A muted, saturated accent from RGBA pixels, or null with none to go on.
Color? ambientFrom(Uint8List rgba) {
  var (r, g, b, weight) = (0.0, 0.0, 0.0, 0.0);
  for (var i = 0; i + 3 < rgba.length; i += 4) {
    final (pr, pg, pb) = (rgba[i], rgba[i + 1], rgba[i + 2]);
    final most = max(pr, max(pg, pb));
    final least = min(pr, min(pg, pb));
    // Favour saturated mid-tones; near-black and blown highlights say little.
    final w =
        ((most - least) / 255 + 0.08) * (most > 24 && least < 240 ? 1 : 0.05);
    r += pr * w;
    g += pg * w;
    b += pb * w;
    weight += w;
  }
  if (weight == 0) return null;
  int channel(double total) => (total / weight).round().clamp(40, 200);
  return Color.fromARGB(255, channel(r), channel(g), channel(b));
}

/// The tint of the image at [url], or null if it can't be had. Remembered, so
/// coming back to a title doesn't ask again.
Future<Color?> ambientColor(String url) async {
  final key = imageCacheKey(url);
  final known = _sampled[key];
  if (known != null) return known;
  final image = await _decodeSmall(sizedImage(url, 48, height: 27));
  if (image == null) return null;
  final bytes = await image.toByteData();
  image.dispose();
  final color = bytes == null ? null : ambientFrom(bytes.buffer.asUint8List());
  if (color != null) _sampled[key] = color;
  return color;
}

Future<ui.Image?> _decodeSmall(String url) {
  final done = Completer<ui.Image?>();
  final stream = ResizeImage(
    CachedNetworkImageProvider(url, cacheKey: imageCacheKey(url)),
    width: 24,
    height: 14,
  ).resolve(ImageConfiguration.empty);
  late final ImageStreamListener listener;
  listener = ImageStreamListener(
    (info, _) {
      if (!done.isCompleted) done.complete(info.image.clone());
      info.dispose();
      stream.removeListener(listener);
    },
    onError: (_, _) {
      if (!done.isCompleted) done.complete(null);
      stream.removeListener(listener);
    },
  );
  stream.addListener(listener);
  return done.future;
}
