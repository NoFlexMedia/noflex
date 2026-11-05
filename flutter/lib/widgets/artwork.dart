// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:io';

import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';

import '../di.dart';
import '../models/work.dart';
import '../services/tokens.dart';
import '../theme.dart';
import '../utils/images.dart';

/// A work's art at the size it's drawn, asking the server to resize it. Falls
/// back to the kind's icon so an unmatched title still reads as something.
class Artwork extends StatelessWidget {
  const Artwork({
    super.key,
    required this.work,
    this.kind = ArtKind.poster,
    this.fit = BoxFit.cover,
    this.radius = NfRadii.m,
    this.localPath,
  });

  final WorkSummary work;
  final ArtKind kind;
  final BoxFit fit;
  final double radius;

  /// An offline copy, preferred over the network.
  final String? localPath;

  @override
  Widget build(BuildContext context) {
    final url = pickImage(work.images, kind);
    final local = localPath;
    return ClipRRect(
      borderRadius: BorderRadius.circular(radius),
      child: ColoredBox(
        color: NfColors.surface2,
        child: LayoutBuilder(
          builder: (context, constraints) {
            final fallback = kind == ArtKind.logo
                ? const SizedBox.shrink()
                : Center(
                    child: Icon(
                      _iconFor(work.kind),
                      size: 28,
                      color: NfColors.text3,
                    ),
                  );
            if (local != null) {
              return Image.file(
                File(local),
                fit: fit,
                width: double.infinity,
                height: double.infinity,
                errorBuilder: (_, _, _) => fallback,
              );
            }
            if (url == null) return fallback;
            final ratio = MediaQuery.devicePixelRatioOf(context);
            final width = constraints.maxWidth.isFinite
                ? constraints.maxWidth * ratio
                : 600.0;
            final absolute = getIt<TokenStore>().absolute(url);
            final sized = sizedImage(absolute, imageWidthStep(width));
            return CachedNetworkImage(
              imageUrl: sized,
              cacheKey: imageCacheKey(sized),
              fit: fit,
              width: double.infinity,
              height: double.infinity,
              fadeInDuration: NfMotion.base,
              // Crossing a step while resizing shouldn't blank the picture.
              useOldImageOnUrlChange: true,
              placeholder: (_, _) => const SizedBox.shrink(),
              errorWidget: (_, _, _) => fallback,
            );
          },
        ),
      ),
    );
  }

  static IconData _iconFor(String kind) => switch (kind) {
    'SERIES' || 'SEASON' || 'EPISODE' => PhosphorIconsRegular.television,
    'ALBUM' || 'TRACK' || 'ARTIST' => PhosphorIconsRegular.musicNotes,
    'AUDIO_BOOK' => PhosphorIconsRegular.bookOpen,
    'PHOTO' => PhosphorIconsRegular.image,
    _ => PhosphorIconsRegular.filmStrip,
  };
}
