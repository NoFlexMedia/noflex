// SPDX-License-Identifier: AGPL-3.0-only

import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter/material.dart';

import '../../di.dart';
import '../../services/tokens.dart';
import '../../theme.dart';
import '../../utils/images.dart';

/// A channel's logo, or a monogram tinted from its name so that a guide full of
/// channels without logos can still be told apart at a glance.
class ChannelLogo extends StatelessWidget {
  const ChannelLogo({
    super.key,
    required this.name,
    this.logoUrl,
    this.size = 40,
  });

  final String name;
  final String? logoUrl;
  final double size;

  @override
  Widget build(BuildContext context) {
    final url = logoUrl;
    final monogram = _Monogram(name: name, size: size);
    return ClipRRect(
      borderRadius: BorderRadius.circular(size * 0.28),
      child: SizedBox.square(
        dimension: size,
        child: url == null || url.isEmpty
            ? monogram
            : ColoredBox(
                color: NfColors.text,
                child: Padding(
                  padding: EdgeInsets.all(size * 0.12),
                  child: CachedNetworkImage(
                    imageUrl: getIt<TokenStore>().absolute(url),
                    cacheKey: imageCacheKey(getIt<TokenStore>().absolute(url)),
                    fit: BoxFit.contain,
                    memCacheWidth:
                        (size * MediaQuery.devicePixelRatioOf(context)).round(),
                    fadeInDuration: NfMotion.fast,
                    placeholder: (_, _) => const SizedBox.shrink(),
                    errorWidget: (_, _, _) => monogram,
                  ),
                ),
              ),
      ),
    );
  }
}

class _Monogram extends StatelessWidget {
  const _Monogram({required this.name, required this.size});
  final String name;
  final double size;

  @override
  Widget build(BuildContext context) {
    final words = RegExp(r'[A-Za-z0-9]+').allMatches(name).take(2);
    final initials = words.isEmpty
        ? '?'
        : words.map((word) => word[0]![0]).join().toUpperCase();
    final hue = name.runes.fold<int>(0, (hash, rune) => hash * 31 + rune) % 360;
    return DecoratedBox(
      decoration: BoxDecoration(
        gradient: LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [
            HSLColor.fromAHSL(1, hue.toDouble(), 0.45, 0.26).toColor(),
            HSLColor.fromAHSL(1, (hue + 40) % 360, 0.40, 0.16).toColor(),
          ],
        ),
      ),
      child: Center(
        child: Text(
          initials,
          style: nfText(
            size * 0.34,
            weight: 750,
            letterSpacing: -0.2,
            color: HSLColor.fromAHSL(1, hue.toDouble(), 0.70, 0.82).toColor(),
          ),
        ),
      ),
    );
  }
}
