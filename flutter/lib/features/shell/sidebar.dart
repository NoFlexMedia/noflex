// SPDX-License-Identifier: AGPL-3.0-only
//
// The navigation with room to spare, as on the web: the same destinations in the
// same order, then the foot with Administration and the server's status. It
// collapses to a strip of icons. Everything centers on the strip's axis, so
// collapsing only clips and fades, and nothing moves.

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/queries/__generated__/server_libraries.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/library_fields.data.gql.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../utils/categories.dart';
import '../../widgets/brand_mark.dart';
import '../../widgets/nf_button.dart';
import '../admin/admin_screen.dart';
import '../browse/category_screen.dart';
import '../browse/collections_screen.dart';
import '../browse/library_screen.dart';
import '../livetv/live_tv_screen.dart';
import '../rooms/watch_rooms_screen.dart';
import 'server_status.dart';

const _insetX = 12.0;
const _itemRadius = 11.0;
const _navMotion = Duration(milliseconds: 280);
const _linkedLibrariesEvery = Duration(minutes: 5);

/// Icons sit on the collapsed strip's axis: half its width, less the padding.
const _iconInset = (NfSpacing.railWidth - _insetX * 2 - 19) / 2;

IconData _libraryIcon(GLibraryFields library) => switch (library.kind.name) {
  'MOVIES' => PhosphorIconsRegular.filmStrip,
  'SHOWS' => PhosphorIconsRegular.television,
  'MUSIC' => PhosphorIconsRegular.musicNotes,
  'AUDIOBOOKS' => PhosphorIconsRegular.bookOpen,
  'PHOTOS' => PhosphorIconsRegular.image,
  'MIXED' => PhosphorIconsRegular.stack,
  _ => PhosphorIconsRegular.books,
};

/// What a linked server answered, or that it didn't.
typedef _LinkedLibraries = ({bool failed, List<GLibraryFields> items});

class Sidebar extends StatefulWidget {
  const Sidebar({super.key});

  @override
  State<Sidebar> createState() => _SidebarState();
}

class _SidebarState extends State<Sidebar> {
  final _linked = signal<Map<String, _LinkedLibraries>>(const {});
  late final void Function() _stopLinked;
  Timer? _linkedTimer;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_onKey);
    final session = getIt<SessionService>();
    _stopLinked = effect(() {
      final servers = session.showLibraries.value
          ? session.connectedServers.value
          : const <ServerRef>[];
      _linkedTimer?.cancel();
      if (servers.isEmpty) return;
      unawaited(_loadLinked(servers));
      _linkedTimer = Timer.periodic(
        _linkedLibrariesEvery,
        (_) => unawaited(_loadLinked(servers)),
      );
    });
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _stopLinked();
    _linkedTimer?.cancel();
    super.dispose();
  }

  Future<void> _loadLinked(List<ServerRef> servers) async {
    final graphql = getIt<GraphqlService>();
    final answers = await Future.wait(
      servers.map((server) async {
        try {
          final data = await graphql.run(
            GServerLibrariesReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
            server: server,
          );
          return (
            server.id,
            (failed: false, items: <GLibraryFields>[...data.libraries]),
          );
        } on Exception {
          // One server being away says nothing about what it held.
          final last = _linked.value[server.id];
          return (
            server.id,
            (failed: true, items: last?.items ?? const <GLibraryFields>[]),
          );
        }
      }),
    );
    if (mounted) _linked.value = {for (final (id, libs) in answers) id: libs};
  }

  /// `[`, as on the web, unless it's being typed or something covers the shell.
  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent ||
        event.logicalKey != LogicalKeyboardKey.bracketLeft) {
      return false;
    }
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isMetaPressed || keys.isAltPressed) {
      return false;
    }
    if (!mounted || !(ModalRoute.isCurrentOf(context) ?? true)) return false;
    final focus = FocusManager.instance.primaryFocus?.context;
    if (focus?.findAncestorStateOfType<EditableTextState>() != null) {
      return false;
    }
    getIt<NavService>().toggleSidebar();
    return true;
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final nav = getIt<NavService>();
      final session = getIt<SessionService>();
      final collapsed = nav.sidebarCollapsed.value;
      final place = nav.place.value;
      final tab = nav.tab.value;
      bool onTab(ShellTab candidate) => place == null && tab == candidate;

      Widget placeItem(String id, String label, IconData icon, Widget page) =>
          _SidebarItem(
            label: label,
            icon: icon,
            collapsed: collapsed,
            selected: place == id,
            onTap: () => nav.open(id, page),
          );
      Widget tabItem(ShellTab candidate, String label, IconData icon) =>
          _SidebarItem(
            label: label,
            icon: icon,
            collapsed: collapsed,
            selected: onTab(candidate),
            onTap: () => nav.selectTab(candidate),
          );
      List<Widget> libraryItems(
        Iterable<GLibraryFields> libraries,
        ServerRef? server,
      ) => [
        for (final library in libraries)
          placeItem(
            NavPlace.library(library.id, server: server?.id),
            library.name,
            _libraryIcon(library),
            LibraryScreen(
              libraryId: library.id,
              title: library.name,
              server: server,
            ),
          ),
      ];

      final linked = _linked.value;
      final connected = session.connectedServers.value;
      return AnimatedContainer(
        duration: MediaQuery.disableAnimationsOf(context)
            ? Duration.zero
            : _navMotion,
        curve: NfMotion.easeOut,
        width: collapsed ? NfSpacing.railWidth : NfSpacing.sidebarWidth,
        clipBehavior: Clip.hardEdge,
        decoration: const BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            stops: [0, 0.4],
            colors: [Color(0x05FFFFFF), Color(0x00FFFFFF)],
          ),
          color: NfColors.sidebar,
          border: Border(right: BorderSide(color: NfColors.line)),
        ),
        child: SafeArea(
          right: false,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _Brand(collapsed: collapsed),
              Expanded(
                child: ListView(
                  padding: const EdgeInsets.symmetric(horizontal: _insetX),
                  children: [
                    tabItem(ShellTab.home, 'Home', PhosphorIconsRegular.house),
                    tabItem(
                      ShellTab.search,
                      'Search',
                      PhosphorIconsRegular.magnifyingGlass,
                    ),
                    for (final category in categories)
                      if (session.categories.value[category.value]
                          case final count?)
                        placeItem(
                          NavPlace.category(category.value.name),
                          category.label,
                          category.icon,
                          CategoryScreen(category: category.withCount(count)),
                        ),
                    placeItem(
                      NavPlace.collections,
                      'Collections',
                      PhosphorIconsRegular.cardsThree,
                      const CollectionsScreen(),
                    ),
                    placeItem(
                      NavPlace.liveTv,
                      'Live TV',
                      PhosphorIconsRegular.television,
                      const LiveTvScreen(),
                    ),
                    placeItem(
                      NavPlace.watchRooms,
                      'Watch together',
                      PhosphorIconsRegular.broadcast,
                      const WatchRoomsScreen(),
                    ),
                    tabItem(
                      ShellTab.downloads,
                      'Downloads',
                      PhosphorIconsRegular.downloadSimple,
                    ),
                    if (session.showLibraries.value) ...[
                      if (session.libraries.value.isNotEmpty) ...[
                        _SidebarHeading(
                          linked.isEmpty
                              ? 'Libraries'
                              : session.serverName.value ?? 'This server',
                          collapsed: collapsed,
                        ),
                        ...libraryItems(session.libraries.value, null),
                      ],
                      for (final server in connected)
                        if (linked[server.id] case final answer?) ...[
                          _SidebarHeading(server.name, collapsed: collapsed),
                          if (answer.failed)
                            _SidebarNote(
                              'Can’t reach this server',
                              collapsed: collapsed,
                            ),
                          ...libraryItems(answer.items, server),
                        ],
                    ],
                  ],
                ),
              ),
              Container(
                margin: const EdgeInsets.symmetric(horizontal: _insetX),
                padding: const EdgeInsets.only(top: 10, bottom: 14),
                decoration: const BoxDecoration(
                  border: Border(top: BorderSide(color: NfColors.line)),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    tabItem(
                      ShellTab.account,
                      session.viewer.value?.name ?? 'You',
                      PhosphorIconsRegular.user,
                    ),
                    // Never lit: it leaves the shell for "Who's watching?".
                    _SidebarItem(
                      label: 'Switch profile',
                      icon: PhosphorIconsRegular.userSwitch,
                      collapsed: collapsed,
                      selected: false,
                      onTap: () => unawaited(session.switchProfile()),
                    ),
                    if (session.viewer.value?.isAdmin ?? false)
                      placeItem(
                        NavPlace.administration,
                        'Administration',
                        PhosphorIconsRegular.shieldCheck,
                        const AdminScreen(),
                      ),
                    const ServerStatus(),
                  ],
                ),
              ),
            ],
          ),
        ),
      );
    },
  );
}

class _Brand extends StatelessWidget {
  const _Brand({required this.collapsed});
  final bool collapsed;

  @override
  Widget build(BuildContext context) {
    final nav = getIt<NavService>();
    return Padding(
      padding: const EdgeInsets.fromLTRB(_insetX, 18, _insetX, 14),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(
            height: 40,
            child: Row(
              children: [
                const SizedBox(
                  width: (NfSpacing.railWidth - _insetX * 2 - 30) / 2,
                ),
                Tooltip(
                  message: 'NoFlex home',
                  child: InkWell(
                    borderRadius: BorderRadius.circular(_itemRadius),
                    onTap: () => nav.selectTab(ShellTab.home),
                    child: const BrandMark(size: 30),
                  ),
                ),
                // Laid out at full width and clipped, so the wordmark and button
                // don't reflow or overflow as the sidebar narrows.
                Expanded(
                  child: ClipRect(
                    child: OverflowBox(
                      alignment: Alignment.centerLeft,
                      minWidth: 171,
                      maxWidth: 171,
                      child: AnimatedOpacity(
                        opacity: collapsed ? 0 : 1,
                        duration: NfMotion.fast,
                        child: Row(
                          children: [
                            const SizedBox(width: 11),
                            Expanded(
                              child: Text(
                                'NoFlex',
                                style: nfText(
                                  17.4,
                                  weight: 750,
                                  letterSpacing: -0.52,
                                ),
                              ),
                            ),
                            if (!collapsed) const _CollapseButton(size: 32),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
          AnimatedSize(
            duration: MediaQuery.disableAnimationsOf(context)
                ? Duration.zero
                : NfMotion.base,
            curve: NfMotion.easeOut,
            alignment: Alignment.topCenter,
            child: collapsed
                ? const Padding(
                    padding: EdgeInsets.only(top: 8),
                    child: Center(child: _CollapseButton(size: 36)),
                  )
                : const SizedBox(width: double.infinity),
          ),
        ],
      ),
    );
  }
}

class _CollapseButton extends SignalWidget {
  const _CollapseButton({required this.size});
  final double size;

  @override
  Widget build(BuildContext context) {
    final nav = getIt<NavService>();
    return NfIconButton(
      icon: PhosphorIconsRegular.sidebarSimple,
      label: nav.sidebarCollapsed.value
          ? 'Expand sidebar  ['
          : 'Collapse sidebar  [',
      size: size,
      iconSize: 19,
      color: NfColors.text2,
      onPressed: nav.toggleSidebar,
    );
  }
}

class _SidebarHeading extends StatelessWidget {
  const _SidebarHeading(this.label, {required this.collapsed});
  final String label;
  final bool collapsed;

  @override
  Widget build(BuildContext context) => SizedBox(
    height: 40,
    child: Stack(
      alignment: Alignment.bottomLeft,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 18, 12, 6),
          child: AnimatedOpacity(
            opacity: collapsed ? 0 : 1,
            duration: NfMotion.fast,
            child: Text(
              label.toUpperCase(),
              maxLines: 1,
              softWrap: false,
              overflow: TextOverflow.ellipsis,
              style: NfType.eyebrow,
            ),
          ),
        ),
        // Headings become a rule, which still divides the icons.
        Positioned(
          left: 10,
          right: 10,
          bottom: 12,
          child: AnimatedOpacity(
            opacity: collapsed ? 1 : 0,
            duration: NfMotion.fast,
            child: const Divider(height: 1),
          ),
        ),
      ],
    ),
  );
}

class _SidebarNote extends StatelessWidget {
  const _SidebarNote(this.text, {required this.collapsed});
  final String text;
  final bool collapsed;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(12, 2, 12, 6),
    child: AnimatedOpacity(
      opacity: collapsed ? 0 : 1,
      duration: NfMotion.fast,
      child: Text(
        text,
        maxLines: 1,
        softWrap: false,
        overflow: TextOverflow.ellipsis,
        style: nfText(11.7, color: NfColors.text3),
      ),
    ),
  );
}

class _SidebarItem extends StatelessWidget {
  const _SidebarItem({
    required this.label,
    required this.icon,
    required this.onTap,
    required this.selected,
    required this.collapsed,
  });

  final String label;
  final IconData icon;
  final VoidCallback onTap;
  final bool selected;
  final bool collapsed;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 1),
    child: Semantics(
      selected: selected,
      button: true,
      label: label,
      excludeSemantics: true,
      child: Tooltip(
        // A label has nowhere else to be, only once it's hidden.
        message: collapsed ? label : '',
        positionDelegate: (at) => Offset(
          at.target.dx + at.targetSize.width / 2 + 10,
          at.target.dy - at.tooltipSize.height / 2,
        ),
        child: Stack(
          clipBehavior: Clip.none,
          children: [
            Material(
              color: selected ? NfColors.surface3 : Colors.transparent,
              borderRadius: BorderRadius.circular(_itemRadius),
              clipBehavior: Clip.antiAlias,
              child: InkWell(
                onTap: onTap,
                hoverColor: NfColors.surface2,
                child: SizedBox(
                  height: 40,
                  child: Row(
                    children: [
                      const SizedBox(width: _iconInset),
                      Icon(
                        icon,
                        size: 19,
                        color: selected ? NfColors.accent : NfColors.text2,
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: AnimatedOpacity(
                          opacity: collapsed ? 0 : 1,
                          duration: NfMotion.fast,
                          child: Text(
                            label,
                            maxLines: 1,
                            softWrap: false,
                            overflow: TextOverflow.ellipsis,
                            style: nfText(
                              13.8,
                              weight: 550,
                              color: selected ? NfColors.text : NfColors.text2,
                            ),
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
            if (selected)
              Positioned(
                left: -_insetX,
                top: 10,
                bottom: 10,
                width: 3,
                child: TweenAnimationBuilder<double>(
                  tween: Tween(begin: 0, end: 1),
                  duration: NfMotion.base,
                  curve: NfMotion.easeOut,
                  builder: (_, scale, child) =>
                      Transform.scale(scaleY: scale, child: child),
                  child: const DecoratedBox(
                    decoration: BoxDecoration(
                      color: NfColors.accent,
                      borderRadius: BorderRadius.horizontal(
                        right: Radius.circular(3),
                      ),
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    ),
  );
}
