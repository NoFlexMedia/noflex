// SPDX-License-Identifier: AGPL-3.0-only

import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter/material.dart';

import '../di.dart';
import '../services/tokens.dart';
import '../theme.dart';
import '../utils/images.dart';

/// "Vera Viewer" is VV, "vera" is VE, as on the other clients.
String initialsOf(String? name) {
  final words = (name ?? '').trim().split(RegExp(r'\s+'))
    ..removeWhere((word) => word.isEmpty);
  if (words.isEmpty) return '';
  final letters = words.length > 1
      ? [words.first, words.last].map((word) => word.characters.first).join()
      : words.first.characters.take(2).toString();
  return letters.toUpperCase();
}

/// Someone's picture, over their initials so a missing or failed one still says
/// who it is. Decorative: whatever's beside it names them.
class UserAvatar extends StatelessWidget {
  const UserAvatar({
    super.key,
    required this.name,
    required this.avatarUrl,
    this.size = 32,
  });

  final String? name;
  final String? avatarUrl;
  final double size;

  @override
  Widget build(BuildContext context) {
    final path = avatarUrl;
    final url = path == null ? null : getIt<TokenStore>().absolute(path);
    return ExcludeSemantics(
      child: Container(
        width: size,
        height: size,
        alignment: Alignment.center,
        clipBehavior: Clip.antiAlias,
        decoration: const BoxDecoration(
          shape: BoxShape.circle,
          gradient: LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [NfColors.accent, Color(0xFF8DE2C5)],
          ),
        ),
        child: Stack(
          alignment: Alignment.center,
          fit: StackFit.expand,
          children: [
            Center(
              child: Text(
                initialsOf(name),
                style: nfText(
                  size * 0.36,
                  weight: 700,
                  color: NfColors.accentInk,
                ),
              ),
            ),
            if (url != null)
              CachedNetworkImage(
                imageUrl: url,
                cacheKey: imageCacheKey(url),
                fit: BoxFit.cover,
                fadeInDuration: NfMotion.base,
                placeholder: (_, _) => const SizedBox.shrink(),
                errorWidget: (_, _, _) => const SizedBox.shrink(),
              ),
          ],
        ),
      ),
    );
  }
}
