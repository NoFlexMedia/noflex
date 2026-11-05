// SPDX-License-Identifier: AGPL-3.0-only
//
// Size classes and fold awareness. Every layout decision goes through these
// rather than through device checks: a tablet, an unfolded phone and a narrow
// desktop window are the same thing to a layout.

import 'dart:math' as math;
import 'dart:ui' show DisplayFeature, DisplayFeatureState, DisplayFeatureType;

import 'package:flutter/material.dart';

/// Material's window size classes, in logical pixels of available width.
enum WindowClass {
  compact,
  medium,
  expanded,
  large;

  static WindowClass ofWidth(double width) => width < 600
      ? compact
      : width < 840
      ? medium
      : width < 1200
      ? expanded
      : large;

  bool get isCompact => this == compact;
  bool get atLeastExpanded => index >= expanded.index;
}

/// The shell's sidebar needs width the content can spare; below this, a rail.
const sidebarMinWidth = 1100.0;

/// A fold or hinge that separates part of the window.
class Fold {
  const Fold(this.bounds, {required this.halfOpen});

  final Rect bounds;

  /// Bent rather than flat: tabletop or book posture.
  final bool halfOpen;

  /// The fold runs top to bottom, so panes sit side by side (book posture).
  bool get isVertical => bounds.height > bounds.width;
}

extension WindowContext on BuildContext {
  WindowClass get windowClass =>
      WindowClass.ofWidth(MediaQuery.sizeOf(this).width);

  /// The page gutter, `clamp(16px, 3.6vw, 52px)` as on the web.
  double get gutter =>
      (MediaQuery.sizeOf(this).width * 0.036).clamp(16.0, 52.0).toDouble();

  /// The fold across this window, if any. Bounds are in window coordinates,
  /// or the content's below a [FoldsFromContent].
  Fold? get fold {
    for (final feature in MediaQuery.displayFeaturesOf(this)) {
      if (feature.type == DisplayFeatureType.cutout) continue;
      return Fold(
        feature.bounds,
        halfOpen: feature.state == DisplayFeatureState.postureHalfOpened,
      );
    }
    return null;
  }
}

/// For content [left] into the window: folds below here are measured from its
/// left edge, so [AdaptivePanes] can split on one without knowing what's beside
/// it. It couldn't find out by measuring: it has no position until after its
/// first layout, and a route's transition moves it for a while after that.
class FoldsFromContent extends StatelessWidget {
  const FoldsFromContent({super.key, required this.left, required this.child});

  final double left;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final media = MediaQuery.of(context);
    return MediaQuery(
      data: media.copyWith(
        displayFeatures: [
          for (final feature in media.displayFeatures)
            DisplayFeature(
              bounds: feature.bounds.shift(Offset(-left, 0)),
              type: feature.type,
              state: feature.state,
            ),
        ],
      ),
      child: child,
    );
  }
}

/// Two panes that respect a fold: they split on it when one runs through them,
/// and otherwise share the width by [primaryFraction]. Below [minWidth] they
/// stack, with the primary pane first.
class AdaptivePanes extends StatelessWidget {
  const AdaptivePanes({
    super.key,
    required this.primary,
    required this.secondary,
    this.primaryFraction = 0.42,
    this.minWidth = 760,
    this.gap = 0,
    this.primaryFits = false,
    this.splitOnFlatFold = true,
  });

  final Widget primary;
  final Widget secondary;
  final double primaryFraction;
  final double minWidth;
  final double gap;

  /// The primary pane is as wide as it asks to be, rather than [primaryFraction],
  /// so it has to size itself to something finite.
  final bool primaryFits;

  /// Whether a fold lying flat still divides the panes. Worth it for two things
  /// of equal standing. A list beside what it opens would get half the screen for
  /// a few words, and a flat fold is no obstacle to what crosses it.
  final bool splitOnFlatFold;

  @override
  Widget build(BuildContext context) {
    final contextFold = context.fold;
    final fold =
        contextFold != null && !contextFold.halfOpen && !splitOnFlatFold
        ? null
        : contextFold;
    return LayoutBuilder(
      builder: (context, constraints) {
        if (constraints.maxWidth < minWidth) {
          return CustomScrollView(
            slivers: [
              SliverToBoxAdapter(child: primary),
              SliverToBoxAdapter(child: secondary),
            ],
          );
        }
        if (primaryFits &&
            !foldSplits(totalWidth: constraints.maxWidth, fold: fold)) {
          return Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              primary,
              SizedBox(width: gap),
              Expanded(child: secondary),
            ],
          );
        }
        final widths = splitAroundFold(
          totalWidth: constraints.maxWidth,
          fold: fold,
          primaryFraction: primaryFraction,
          gap: gap,
        );
        return Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SizedBox(width: widths.primary, child: primary),
            SizedBox(width: widths.gap),
            Expanded(child: secondary),
          ],
        );
      },
    );
  }
}

/// A vertical fold in the middle of the content, rather than near an edge (the
/// shell's navigation, say).
bool foldSplits({required double totalWidth, required Fold? fold}) {
  if (fold == null || !fold.isVertical) return false;
  final left = fold.bounds.left;
  return left > totalWidth * 0.25 && left < totalWidth * 0.75;
}

/// How wide each pane is. Keeps a pane from straddling a vertical fold that
/// falls inside the middle of the content; a fold near an edge (the shell's
/// navigation, say) is ignored so the panes aren't squeezed.
({double primary, double gap}) splitAroundFold({
  required double totalWidth,
  required Fold? fold,
  required double primaryFraction,
  required double gap,
}) {
  if (foldSplits(totalWidth: totalWidth, fold: fold)) {
    return (primary: fold!.bounds.left, gap: math.max(fold.bounds.width, gap));
  }
  return (primary: totalWidth * primaryFraction, gap: gap);
}

/// Columns of a poster grid: `minmax(clamp(132px, 15vw, 186px), 1fr)`.
int posterColumns(double width) {
  final minCell = (width * 0.15).clamp(132.0, 186.0);
  return math.max(2, (width / (minCell + 14)).floor());
}
