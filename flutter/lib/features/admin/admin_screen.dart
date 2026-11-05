// SPDX-License-Identifier: AGPL-3.0-only
//
// The server's administration: a list of sections that sits beside the open one
// when there's room, and leads to it when there isn't. Which section is open
// stays a signal rather than a route, so unfolding a phone keeps the place.

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_state.dart';
import '../browse/work_browser.dart';
import 'admin_activity.dart';
import 'admin_controller.dart';
import 'admin_indexing.dart';
import 'admin_libraries.dart';
import 'admin_logs.dart';
import 'admin_overview.dart';
import 'admin_playback.dart';
import 'admin_plugins.dart';
import 'admin_resources.dart';
import 'admin_server.dart';
import 'admin_transcoding.dart';
import 'admin_users.dart';

const _twoPaneWidth = 760.0;
const _sideNavWidth = 260.0;

class AdminScreen extends StatefulWidget {
  const AdminScreen({super.key});

  @override
  State<AdminScreen> createState() => _AdminScreenState();
}

class _AdminScreenState extends State<AdminScreen> {
  // Only an administrator gets past the guard below, and everything here asks
  // the server for what only they can read.
  AdminController? _controller;

  @override
  void dispose() {
    _controller?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      if (getIt<SessionService>().viewer.value?.isAdmin != true) {
        return Scaffold(
          appBar: const NfPageBar(title: 'Administration'),
          body: NfState(
            icon: PhosphorIconsRegular.shieldCheck,
            title: 'Administrators only',
            message: 'This area manages the whole server. Ask an administrator if you need something changed.',
            child: Padding(
              padding: const EdgeInsets.only(top: 16),
              child: NfButton(
                label: 'Go home',
                onPressed: () => getIt<NavService>().selectTab(ShellTab.home),
              ),
            ),
          ),
        );
      }
      final controller = _controller ??= AdminController();
      return LayoutBuilder(
        builder: (context, constraints) {
          final width = constraints.maxWidth;
          if (width >= _twoPaneWidth) {
            return Scaffold(
              appBar: const NfPageBar(title: 'Administration'),
              body: AdaptivePanes(
                primaryFraction: (_sideNavWidth / width).clamp(0.2, 0.42),
                minWidth: _twoPaneWidth,
                primary: _SectionList(controller: controller, panes: true),
                secondary: SignalBuilder(
                  builder: (context) => _SectionPane(
                    controller: controller,
                    section: controller.section.value ?? AdminSection.overview,
                  ),
                ),
              ),
            );
          }
          return SignalBuilder(
            builder: (context) {
              final section = controller.section.value;
              if (section == null) {
                return Scaffold(
                  appBar: const NfPageBar(title: 'Administration'),
                  body: _SectionList(controller: controller, panes: false),
                );
              }
              return PopScope(
                canPop: false,
                onPopInvokedWithResult: (didPop, _) {
                  if (!didPop) controller.section.value = null;
                },
                child: Scaffold(
                  appBar: AppBar(
                    backgroundColor: NfColors.bg,
                    surfaceTintColor: Colors.transparent,
                    foregroundColor: NfColors.text,
                    leading: BackButton(
                      onPressed: () => controller.section.value = null,
                    ),
                    title: Text(
                      section.label,
                      style: nfText(17, weight: 650, letterSpacing: -0.3),
                    ),
                  ),
                  body: _SectionPane(controller: controller, section: section),
                ),
              );
            },
          );
        },
      );
    },
  );
}

class _SectionList extends SignalWidget {
  const _SectionList({required this.controller, required this.panes});
  final AdminController controller;

  /// Beside the section, so one is always selected.
  final bool panes;

  @override
  Widget build(BuildContext context) {
    final data = controller.admin.data.value;
    final info = data?.serverInfo;
    final selected = panes
        ? controller.section.value ?? AdminSection.overview
        : null;
    int? countOf(AdminSection section) => switch (section) {
      AdminSection.libraries => data?.libraries.length,
      AdminSection.plugins => data?.plugins.length,
      AdminSection.users => data?.users.length,
      _ => null,
    };
    return DecoratedBox(
      decoration: BoxDecoration(
        border: panes
            ? const Border(right: BorderSide(color: NfColors.line))
            : null,
      ),
      child: ListView(
        padding: const EdgeInsets.fromLTRB(16, 12, 12, 24),
        children: [
          if (info != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(10, 0, 10, 14),
              child: Text(
                '${info.name} · v${info.version} · ${info.backend} storage',
                style: NfType.caption,
              ),
            ),
          if (data == null && controller.admin.error.value != null)
            NfState.error(
              title: 'Couldn’t read the server’s state',
              message: controller.admin.error.value.toString(),
              retry: controller.admin.refetch,
            ),
          for (final section in AdminSection.values)
            _SectionItem(
              section: section,
              selected: section == selected,
              count: countOf(section),
              chevron: !panes,
              onTap: () => controller.open(section),
            ),
        ],
      ),
    );
  }
}

class _SectionItem extends StatelessWidget {
  const _SectionItem({
    required this.section,
    required this.selected,
    required this.count,
    required this.chevron,
    required this.onTap,
  });

  final AdminSection section;
  final bool selected;
  final int? count;
  final bool chevron;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 1),
    child: Semantics(
      selected: selected,
      button: true,
      child: Material(
        color: selected ? NfColors.surface3 : Colors.transparent,
        borderRadius: BorderRadius.circular(NfRadii.s),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          child: Padding(
            padding: EdgeInsets.symmetric(
              horizontal: 10,
              vertical: chevron ? 13 : 10,
            ),
            child: Row(
              children: [
                Icon(
                  section.icon,
                  size: 19,
                  color: selected ? NfColors.accent : NfColors.text2,
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Text(
                    section.label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: nfText(
                      14,
                      weight: selected ? 600 : 500,
                      color: selected ? NfColors.text : NfColors.text2,
                    ),
                  ),
                ),
                if (count != null) Text('$count', style: NfType.caption),
                if (chevron) ...[
                  const SizedBox(width: 6),
                  const Icon(
                    PhosphorIconsRegular.caretRight,
                    size: 15,
                    color: NfColors.text3,
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    ),
  );
}

class _SectionPane extends StatelessWidget {
  const _SectionPane({required this.controller, required this.section});
  final AdminController controller;
  final AdminSection section;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return SingleChildScrollView(
      // A fresh position per section: one's scroll offset means nothing to the
      // next.
      key: ValueKey(section),
      padding: EdgeInsets.fromLTRB(gutter, 16, gutter, 32),
      child: switch (section) {
        AdminSection.overview => OverviewSection(controller: controller),
        AdminSection.activity => ActivitySection(controller: controller),
        AdminSection.playback => const PlaybackSection(),
        AdminSection.libraries => LibrariesSection(controller: controller),
        AdminSection.indexing => IndexingSection(controller: controller),
        AdminSection.transcoding => const TranscodingSection(),
        AdminSection.resources => ResourcesSection(live: controller.live),
        AdminSection.plugins => PluginsSection(controller: controller),
        AdminSection.users => UsersSection(controller: controller),
        AdminSection.logs => const LogsSection(),
        AdminSection.server => ServerSection(controller: controller),
      },
    );
  }
}
