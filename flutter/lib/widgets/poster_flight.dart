// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:ui' show lerpDouble;

import 'package:flutter/material.dart';

import '../models/work.dart';
import '../utils/images.dart';
import 'artwork.dart';

/// What a card hands the page it opens, to carry its picture across.
final class PosterFlight {
  const PosterFlight({required this.tag, required this.width});

  /// One card's own, since a title can be on a page in several rails at once.
  final Object tag;

  /// How wide the card draws it, so the page can show that picture until its
  /// own size arrives.
  final double width;
}

/// [Artwork] that flies between a card and the page it opens when both name the
/// same [tag]; with no tag, it stays put.
class PosterHero extends StatelessWidget {
  const PosterHero({
    super.key,
    required this.tag,
    required this.work,
    required this.kind,
    required this.radius,
    this.previewWidth,
  });

  final Object? tag;
  final WorkSummary work;
  final ArtKind kind;
  final double radius;
  final double? previewWidth;

  @override
  Widget build(BuildContext context) {
    final art = Artwork(
      work: work,
      kind: kind,
      radius: radius,
      previewWidth: previewWidth,
    );
    final heroTag = tag;
    if (heroTag == null) return art;
    return HeroMode(
      enabled: !MediaQuery.disableAnimationsOf(context),
      child: Hero(tag: heroTag, flightShuttleBuilder: _shuttle, child: art),
    );
  }

  /// Scales the card's picture instead of laying art out at every size on the
  /// way. That picture is already in memory, and each size would be a download.
  static Widget _shuttle(
    BuildContext _,
    Animation<double> animation,
    HeroFlightDirection direction,
    BuildContext from,
    BuildContext to,
  ) {
    final card = direction == HeroFlightDirection.push ? from : to;
    final page = direction == HeroFlightDirection.push ? to : from;
    final cardArt = (card.widget as Hero).child as Artwork;
    final pageArt = (page.widget as Hero).child as Artwork;
    return AnimatedBuilder(
      animation: animation,
      builder: (_, child) => ClipRRect(
        borderRadius: BorderRadius.circular(
          lerpDouble(cardArt.radius, pageArt.radius, animation.value)!,
        ),
        child: child,
      ),
      child: FittedBox(
        fit: BoxFit.cover,
        child: SizedBox.fromSize(
          size: (card.findRenderObject()! as RenderBox).size,
          child: Artwork(work: cardArt.work, kind: cardArt.kind, radius: 0),
        ),
      ),
    );
  }
}
