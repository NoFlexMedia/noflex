// SPDX-License-Identifier: AGPL-3.0-only

import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter/material.dart';
import 'package:jovial_svg/jovial_svg.dart';

import '../di.dart';
import '../graphql/queries/fragments/__generated__/provider_attribution_fields.data.gql.dart';
import '../services/tokens.dart';

/// A provider's logo at [height], as the provider made it: their terms forbid
/// recolouring these, so no tint. Nothing at all without one, or while it can't
/// be fetched, since the credit's text still says who it is.
class ProviderLogo extends StatelessWidget {
  const ProviderLogo({
    super.key,
    required this.credit,
    this.height = 24,
    this.server,
  });

  final GProviderAttributionFields credit;
  final double height;

  /// The origin of the linked server the credit came from, if not the home one.
  final String? server;

  /// Without one, every rebuild fetches and parses the logo again.
  static final _cache = ScalableImageCache(size: 8);

  @override
  Widget build(BuildContext context) {
    final path = credit.logoUrl;
    if (path == null) return const SizedBox.shrink();
    final url = server != null && path.startsWith('/')
        ? '$server$path'
        : getIt<TokenStore>().absolute(path);
    return Semantics(
      label: credit.name,
      image: true,
      child: credit.logoMime == 'image/svg+xml'
          ? SizedBox(
              height: height,
              child: ScalableImageWidget.fromSISource(
                si: ScalableImageSource.fromSvgHttpUrl(Uri.parse(url)),
                cache: _cache,
                onLoading: (_) => const SizedBox.shrink(),
                onError: (_) => const SizedBox.shrink(),
                // Left to itself the widget takes the width the document
                // declares, not what its height leaves.
                onLoaded: (_, image) => AspectRatio(
                  aspectRatio: image.viewport.width / image.viewport.height,
                  child: ScalableImageWidget(si: image),
                ),
              ),
            )
          : CachedNetworkImage(
              imageUrl: url,
              height: height,
              fit: BoxFit.contain,
              fadeInDuration: Duration.zero,
              errorWidget: (_, _, _) => const SizedBox.shrink(),
            ),
    );
  }
}
