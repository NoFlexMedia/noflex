// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../utils/categories.dart';
import '../../widgets/nf_state.dart';
import '../livetv/live_tv_screen.dart';
import '../rooms/watch_rooms_screen.dart';
import 'category_screen.dart';
import 'collections_screen.dart';
import 'library_screen.dart';

class BrowseScreen extends SignalWidget {
  const BrowseScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final session = getIt<SessionService>();
    final nav = getIt<NavService>();
    final available = [
      for (final category in categories)
        if (session.categories.value.containsKey(category.value))
          category.withCount(session.categories.value[category.value]!),
    ];
    final libraries = session.libraries.value;
    final gutter = context.gutter;
    return SafeArea(
      child: CustomScrollView(
        slivers: [
          SliverPadding(
            padding: EdgeInsets.fromLTRB(gutter, 20, gutter, 16),
            sliver: SliverToBoxAdapter(
              child: Text('Browse', style: NfType.pageTitle),
            ),
          ),
          SliverPadding(
            padding: EdgeInsets.symmetric(horizontal: gutter),
            sliver: SliverGrid.extent(
              maxCrossAxisExtent: 240,
              mainAxisSpacing: 12,
              crossAxisSpacing: 12,
              childAspectRatio: 1.7,
              children: [
                for (final category in available)
                  _Tile(
                    label: category.label,
                    detail: '${category.workCount}',
                    icon: category.icon,
                    onTap: () => nav.push(CategoryScreen(category: category)),
                  ),
              ],
            ),
          ),
          SliverPadding(
            padding: EdgeInsets.fromLTRB(gutter, 28, gutter, 12),
            sliver: const SliverToBoxAdapter(child: NfSectionHead('More')),
          ),
          SliverPadding(
            padding: EdgeInsets.symmetric(horizontal: gutter),
            sliver: SliverGrid.extent(
              maxCrossAxisExtent: 240,
              mainAxisSpacing: 12,
              crossAxisSpacing: 12,
              childAspectRatio: 1.7,
              children: [
                _Tile(
                  label: 'Collections',
                  icon: PhosphorIconsRegular.cardsThree,
                  onTap: () => nav.push(const CollectionsScreen()),
                ),
                _Tile(
                  label: 'Live TV',
                  icon: PhosphorIconsRegular.television,
                  onTap: () => nav.push(const LiveTvScreen()),
                ),
                _Tile(
                  label: 'Watch rooms',
                  icon: PhosphorIconsRegular.broadcast,
                  onTap: () => nav.push(const WatchRoomsScreen()),
                ),
              ],
            ),
          ),
          if (libraries.isNotEmpty) ...[
            SliverPadding(
              padding: EdgeInsets.fromLTRB(gutter, 28, gutter, 12),
              sliver: const SliverToBoxAdapter(
                child: NfSectionHead('Libraries'),
              ),
            ),
            SliverPadding(
              padding: EdgeInsets.fromLTRB(gutter, 0, gutter, 32),
              sliver: SliverGrid.extent(
                maxCrossAxisExtent: 240,
                mainAxisSpacing: 12,
                crossAxisSpacing: 12,
                childAspectRatio: 1.7,
                children: [
                  for (final library in libraries)
                    _Tile(
                      label: library.name,
                      icon: PhosphorIconsRegular.folderSimple,
                      onTap: () => nav.push(
                        LibraryScreen(
                          libraryId: library.id,
                          title: library.name,
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class _Tile extends StatelessWidget {
  const _Tile({
    required this.label,
    required this.icon,
    required this.onTap,
    this.detail,
  });

  final String label;
  final String? detail;
  final IconData icon;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => NfPanel(
    onTap: onTap,
    padding: const EdgeInsets.all(14),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisAlignment: MainAxisAlignment.spaceBetween,
      children: [
        Icon(icon, size: 22, color: NfColors.accent),
        Row(
          children: [
            Expanded(
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: nfText(14.5, weight: 600),
              ),
            ),
            if (detail != null) Text(detail!, style: NfType.caption),
          ],
        ),
      ],
    ),
  );
}
