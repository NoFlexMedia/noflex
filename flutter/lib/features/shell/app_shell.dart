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
import '../../theme.dart';
import '../../widgets/brand_mark.dart';
import '../account/account_screen.dart';
import '../connect/remote_bar.dart';
import '../browse/browse_screen.dart';
import '../downloads/downloads_screen.dart';
import '../home/home_screen.dart';
import '../search/search_screen.dart';
import 'sidebar.dart';

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

class AppShell extends SignalWidget {
  const AppShell({super.key});

  @override
  Widget build(BuildContext context) {
    final width = MediaQuery.sizeOf(context).width;
    final compact = WindowClass.ofWidth(width).isCompact;
    final navigationWidth = compact
        ? 0.0
        : width < sidebarMinWidth || getIt<NavService>().sidebarCollapsed.value
        ? NfSpacing.railWidth
        : NfSpacing.sidebarWidth;
    return Scaffold(
      resizeToAvoidBottomInset: false,
      bottomNavigationBar: compact
          ? const Column(
              mainAxisSize: MainAxisSize.min,
              children: [RemoteBar(), _BottomNav()],
            )
          : null,
      body: Stack(
        children: [
          Row(
            children: [
              if (!compact && width < sidebarMinWidth) const _Rail(),
              if (width >= sidebarMinWidth) const Sidebar(),
              Expanded(
                child: FoldsFromContent(
                  left: navigationWidth,
                  child: const _Content(),
                ),
              ),
            ],
          ),
          // Over the content, beside the navigation; on a phone it sits above the bar instead.
          if (!compact)
            Positioned(
              left: navigationWidth + 8,
              right: 8,
              bottom: 0,
              child: const RemoteBar(),
            ),
        ],
      ),
    );
  }
}

class _Content extends StatefulWidget {
  const _Content();

  @override
  State<_Content> createState() => _ContentState();
}

class _ContentState extends State<_Content> {
  // MaterialApp only gives its own navigator one, and heroes need it to fly.
  final _heroes = MaterialApp.createMaterialHeroController();

  @override
  void dispose() {
    _heroes.dispose();
    super.dispose();
  }

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
    child: HeroControllerScope(
      controller: _heroes,
      child: Navigator(
        key: contentNavigatorKey,
        observers: [getIt<NavService>().placeObserver],
        onGenerateRoute: (settings) => MaterialPageRoute<void>(
          settings: settings,
          builder: (_) => const _TabStack(),
        ),
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
    final current = nav.shownTab.value;
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
    final current = nav.shownTab.value;
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
