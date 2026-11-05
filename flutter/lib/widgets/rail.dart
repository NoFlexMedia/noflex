// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';

import '../layout/window.dart';
import '../models/work.dart';
import 'nf_state.dart';
import 'work_card.dart';

/// A titled horizontal shelf of cards. Card size follows the window, so an
/// unfolded phone shows more of them rather than bigger ones.
class Rail extends StatelessWidget {
  const Rail({
    super.key,
    required this.title,
    required this.items,
    this.eyebrow,
    this.wide = false,
    this.onHeader,
  });

  final String title;
  final String? eyebrow;
  final List<WorkSummary> items;
  final bool wide;
  final VoidCallback? onHeader;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    final width = MediaQuery.sizeOf(context).width;
    final cardWidth = wide
        ? (width * 0.72).clamp(240.0, 340.0)
        : (width * 0.34).clamp(116.0, 176.0);
    final height = wide ? cardWidth * 9 / 16 : cardWidth * 3 / 2 + 46;
    return Padding(
      padding: const EdgeInsets.only(top: 28),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: EdgeInsets.symmetric(horizontal: gutter),
            child: NfSectionHead(title, eyebrow: eyebrow, onTap: onHeader),
          ),
          const SizedBox(height: 6),
          SizedBox(
            height: height + 6,
            child: ListView.separated(
              scrollDirection: Axis.horizontal,
              // Room for a hovered card to lift inside the list's clip.
              padding: EdgeInsets.fromLTRB(gutter, 6, gutter, 0),
              itemCount: items.length,
              separatorBuilder: (_, _) => const SizedBox(width: 12),
              itemBuilder: (_, index) => SizedBox(
                width: cardWidth,
                child: WorkCard(work: items[index], wide: wide),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Skeleton of a rail, while a shelf loads.
class RailSkeleton extends StatelessWidget {
  const RailSkeleton({super.key});

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return Padding(
      padding: EdgeInsets.fromLTRB(gutter, 28, 0, 0),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const NfSkeleton(width: 160, height: 18, radius: 6),
          const SizedBox(height: 14),
          SizedBox(
            height: 190,
            child: ListView.separated(
              scrollDirection: Axis.horizontal,
              physics: const NeverScrollableScrollPhysics(),
              itemCount: 8,
              separatorBuilder: (_, _) => const SizedBox(width: 12),
              itemBuilder: (_, _) => const NfSkeleton(width: 126),
            ),
          ),
        ],
      ),
    );
  }
}
