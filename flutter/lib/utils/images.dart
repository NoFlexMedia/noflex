// SPDX-License-Identifier: AGPL-3.0-only
//
// Image URLs carry a user-bound media token, stable for about an hour. The
// server resizes on `w`/`h`/`fill`.

enum ArtKind { poster, backdrop, thumb, logo }

const _preference = {
  ArtKind.poster: ['PRIMARY', 'BOX', 'THUMB', 'ART'],
  ArtKind.backdrop: ['BACKDROP', 'ART', 'BANNER', 'THUMB', 'PRIMARY'],
  ArtKind.thumb: ['THUMB', 'BACKDROP', 'PRIMARY'],
  ArtKind.logo: ['LOGO'],
};

class ArtImage {
  const ArtImage(this.kind, this.url);
  factory ArtImage.fromJson(Map<String, dynamic> json) =>
      ArtImage(json['kind'] as String, json['url'] as String);
  final String kind;
  final String url;
  Map<String, dynamic> toJson() => {'kind': kind, 'url': url};
}

String? pickImage(List<ArtImage> images, ArtKind kind) {
  if (images.isEmpty) return null;
  for (final name in _preference[kind]!) {
    for (final image in images) {
      if (image.kind == name) return image.url;
    }
  }
  return kind == ArtKind.logo ? null : images.first.url;
}

/// What to ask the server for at [width]. In steps, since every distinct width
/// is its own download and resize, and a window or sidebar moving through
/// hundreds of them in a moment shouldn't mean hundreds of requests.
double imageWidthStep(double width) => (width / 120).ceil() * 120.0;

String sizedImage(String url, double width, {double? height}) {
  final params =
      'w=${width.round()}${height != null ? '&h=${height.round()}&fill=true' : ''}';
  return '$url${url.contains('?') ? '&' : '?'}$params';
}

/// The token changes hourly while the picture doesn't, so it stays out of the
/// disk cache's key.
String imageCacheKey(String url) =>
    url.replaceFirst(RegExp(r'[?&]token=[^&]*'), '');
