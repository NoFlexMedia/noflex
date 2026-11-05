// SPDX-License-Identifier: AGPL-3.0-only
//
// Navigation follows available width, as on the web: a bottom bar on phones, a
// rail on tablets and unfolded foldables, a sidebar with room to spare. Tab
// roots live in an IndexedStack so they keep their state; pages push onto one
// content navigator beside the navigation, never over it.

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../utils/categories.dart';
import '../../widgets/brand_mark.dart';
import '../account/account_screen.dart';
import '../admin/admin_screen.dart';
import '../browse/browse_screen.dart';
import '../browse/category_screen.dart';
import '../browse/collections_screen.dart';
import '../downloads/downloads_screen.dart';
import '../home/home_screen.dart';
import '../livetv/live_tv_screen.dart';
import '../rooms/watch_rooms_screen.dart';
import '../search/search_screen.dart';

const _destinations = [
  (
    tab: ShellTab.home,
    label: 'Home',
    icon: PhosphorIconsRegular.house,
    active: PhosphorIconsFill.house,
  ),
  (
    tab: ShellTab.browse,
    label: 'Browse',
    icon: PhosphorIconsRegular.books,
    active: PhosphorIconsFill.books,
  ),
  (
    tab: ShellTab.search,
    label: 'Search',
    icon: PhosphorIconsRegular.magnifyingGlass,
    active: PhosphorIconsBold.magnifyingGlass,
  ),
  (
    tab: ShellTab.downloads,
    label: 'Downloads',
    icon: PhosphorIconsRegular.downloadSimple,
    active: PhosphorIconsBold.downloadSimple,
  ),
  (
    tab: ShellTab.account,
    label: 'You',
    icon: PhosphorIconsRegular.user,
    active: PhosphorIconsFill.user,
  ),
];

/// Rail below this; the sidebar needs width the content can spare.
const _sidebarMinWidth = 1100.0;

class AppShell extends StatelessWidget {
  const AppShell({super.key});

  @override
  Widget build(BuildContext context) {
    final width = MediaQuery.sizeOf(context).width;
    final compact = WindowClass.ofWidth(width).isCompact;
    final navigationWidth = compact
        ? 0.0
        : width < _sidebarMinWidth
        ? NfSpacing.railWidth
        : NfSpacing.sidebarWidth;
    return Scaffold(
      resizeToAvoidBottomInset: false,
      bottomNavigationBar: compact ? const _BottomNav() : null,
      body: Row(
        children: [
          if (!compact && width < _sidebarMinWidth) const _Rail(),
          if (width >= _sidebarMinWidth) const _Sidebar(),
          Expanded(
            child: FoldsFromContent(
              left: navigationWidth,
              child: const _Content(),
            ),
          ),
        ],
      ),
    );
  }
}

class _Content extends StatelessWidget {
  const _Content();

  @override
  Widget build(BuildContext context) => NavigatorPopHandler(
    onPopWithResult: (_) {
      final nav = getIt<NavService>();
      if (contentNavigatorKey.currentState?.canPop() ?? false) {
        contentNavigatorKey.currentState!.pop();
      } else if (nav.tab.value != ShellTab.home) {
        nav.tab.value = ShellTab.home;
      }
    },
    child: Navigator(
      key: contentNavigatorKey,
      onGenerateRoute: (settings) => MaterialPageRoute<void>(
        settings: settings,
        builder: (_) => const _TabStack(),
      ),
    ),
  );
}

class _TabStack extends SignalWidget {
  const _TabStack();

  @override
  Widget build(BuildContext context) {
    final tab = getIt<NavService>().tab.value;
    return IndexedStack(
      index: tab.index,
      children: const [
        HomeScreen(),
        BrowseScreen(),
        SearchScreen(),
        DownloadsScreen(),
        AccountScreen(),
      ],
    );
  }
}

class _BottomNav extends SignalWidget {
  const _BottomNav();

  @override
  Widget build(BuildContext context) {
    final nav = getIt<NavService>();
    final current = nav.tab.value;
    return Container(
      decoration: const BoxDecoration(
        color: Color(0xF0080A0C),
        border: Border(top: BorderSide(color: NfColors.line)),
      ),
      padding: EdgeInsets.only(bottom: MediaQuery.paddingOf(context).bottom),
      child: SizedBox(
        height: NfSpacing.bottomNavHeight,
        child: Row(
          children: [
            for (final destination in _destinations)
              Expanded(
                child: Semantics(
                  selected: destination.tab == current,
                  button: true,
                  label: destination.label,
                  excludeSemantics: true,
                  child: InkResponse(
                    onTap: () => nav.selectTab(destination.tab),
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Icon(
                          destination.tab == current
                              ? destination.active
                              : destination.icon,
                          size: 22,
                          color: destination.tab == current
                              ? NfColors.accent
                              : NfColors.text3,
                        ),
                        const SizedBox(height: 3),
                        Text(
                          destination.label,
                          style: nfText(
                            10.5,
                            weight: 600,
                            letterSpacing: 0,
                            color: destination.tab == current
                                ? NfColors.accent
                                : NfColors.text3,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _Rail extends SignalWidget {
  const _Rail();

  @override
  Widget build(BuildContext context) {
    final nav = getIt<NavService>();
    final current = nav.tab.value;
    return Container(
      width: NfSpacing.railWidth,
      decoration: const BoxDecoration(
        color: NfColors.bgRaised,
        border: Border(right: BorderSide(color: NfColors.line)),
      ),
      child: SafeArea(
        right: false,
        child: Column(
          children: [
            const SizedBox(height: 16),
            const BrandMark(size: 32),
            const SizedBox(height: 18),
            for (final destination in _destinations)
              _RailItem(
                label: destination.label,
                icon: destination.tab == current
                    ? destination.active
                    : destination.icon,
                selected: destination.tab == current,
                onTap: () => nav.selectTab(destination.tab),
              ),
          ],
        ),
      ),
    );
  }
}

class _RailItem extends StatelessWidget {
  const _RailItem({
    required this.label,
    required this.icon,
    required this.selected,
    required this.onTap,
  });

  final String label;
  final IconData icon;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    selected: selected,
    button: true,
    label: label,
    excludeSemantics: true,
    child: InkWell(
      onTap: onTap,
      child: Container(
        width: double.infinity,
        padding: const EdgeInsets.symmetric(vertical: 10),
        child: Column(
          children: [
            Container(
              width: 52,
              height: 30,
              decoration: BoxDecoration(
                color: selected ? NfColors.accentSoft : Colors.transparent,
                borderRadius: BorderRadius.circular(NfRadii.pill),
              ),
              child: Icon(
                icon,
                size: 21,
                color: selected ? NfColors.accent : NfColors.text2,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              label,
              style: nfText(
                10.5,
                weight: 600,
                letterSpacing: 0,
                color: selected ? NfColors.accent : NfColors.text3,
              ),
            ),
          ],
        ),
      ),
    ),
  );
}

class _Sidebar extends SignalWidget {
  const _Sidebar();

  @override
  Widget build(BuildContext context) {
    final nav = getIt<NavService>();
    final session = getIt<SessionService>();
    final current = nav.tab.value;
    final available = [
      for (final category in categories)
        if (session.categories.value.containsKey(category.value))
          category.withCount(session.categories.value[category.value]!),
    ];
    return Container(
      width: NfSpacing.sidebarWidth,
      decoration: const BoxDecoration(
        color: NfColors.bgRaised,
        border: Border(right: BorderSide(color: NfColors.line)),
      ),
      child: SafeArea(
        right: false,
        child: ListView(
          padding: const EdgeInsets.fromLTRB(12, 18, 12, 18),
          children: [
            Padding(
              padding: const EdgeInsets.only(left: 8, bottom: 18),
              child: Row(
                children: [
                  const BrandMark(size: 28),
                  const SizedBox(width: 10),
                  Text(
                    'NoFlex',
                    style: nfText(17, weight: 700, letterSpacing: -0.4),
                  ),
                ],
              ),
            ),
            for (final destination in _destinations)
              if (destination.tab != ShellTab.account)
                _SidebarItem(
                  label: destination.label,
                  icon: destination.tab == current
                      ? destination.active
                      : destination.icon,
                  selected: destination.tab == current,
                  onTap: () => nav.selectTab(destination.tab),
                ),
            if (available.isNotEmpty) const _SidebarHeading('Library'),
            for (final category in available)
              _SidebarItem(
                label: category.label,
                icon: category.icon,
                onTap: () => nav.push(CategoryScreen(category: category)),
              ),
            const _SidebarHeading('More'),
            _SidebarItem(
              label: 'Collections',
              icon: PhosphorIconsRegular.cardsThree,
              onTap: () => nav.push(const CollectionsScreen()),
            ),
            _SidebarItem(
              label: 'Live TV',
              icon: PhosphorIconsRegular.television,
              onTap: () => nav.push(const LiveTvScreen()),
            ),
            _SidebarItem(
              label: 'Watch rooms',
              icon: PhosphorIconsRegular.broadcast,
              onTap: () => nav.push(const WatchRoomsScreen()),
            ),
            if (session.viewer.value?.isAdmin ?? false)
              _SidebarItem(
                label: 'Administration',
                icon: PhosphorIconsRegular.shieldCheck,
                onTap: () => nav.push(const AdminScreen()),
              ),
            const SizedBox(height: 12),
            _SidebarItem(
              label: session.viewer.value?.name ?? 'You',
              icon: current == ShellTab.account
                  ? PhosphorIconsFill.user
                  : PhosphorIconsRegular.user,
              selected: current == ShellTab.account,
              onTap: () => nav.selectTab(ShellTab.account),
            ),
          ],
        ),
      ),
    );
  }
}

class _SidebarHeading extends StatelessWidget {
  const _SidebarHeading(this.label);
  final String label;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(10, 20, 10, 6),
    child: Text(label.toUpperCase(), style: NfType.eyebrow),
  );
}

class _SidebarItem extends StatelessWidget {
  const _SidebarItem({
    required this.label,
    required this.icon,
    required this.onTap,
    this.selected = false,
  });

  final String label;
  final IconData icon;
  final VoidCallback onTap;
  final bool selected;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 1),
    child: Material(
      color: selected ? NfColors.surface3 : Colors.transparent,
      borderRadius: BorderRadius.circular(NfRadii.s),
      clipBehavior: Clip.antiAlias,
      child: InkWell(
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 9),
          child: Row(
            children: [
              Icon(
                icon,
                size: 19,
                color: selected ? NfColors.accent : NfColors.text2,
              ),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: nfText(
                    14,
                    weight: selected ? 600 : 500,
                    color: selected ? NfColors.text : NfColors.text2,
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    ),
  );
}
