// SPDX-License-Identifier: AGPL-3.0-only
//
// Sections beside the navigation with room for two panes, one above the other
// without. API keys and integrations stay on the web.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_state.dart';
import '../downloads/leave_account.dart';
import 'account_controller.dart';
import 'devices_section.dart';
import 'exports_section.dart';
import 'history_section.dart';
import 'profile_section.dart';
import 'servers_section.dart';

const _twoPaneWidth = 760.0;
const _sideNavWidth = 300.0;

class AccountScreen extends StatefulWidget {
  const AccountScreen({super.key});

  @override
  State<AccountScreen> createState() => _AccountScreenState();
}

class _AccountScreenState extends State<AccountScreen> {
  final _controller = AccountController();
  late final void Function() _stopVisits;

  @override
  void initState() {
    super.initState();
    _stopVisits = effect(() {
      if (getIt<NavService>().tab.value == ShellTab.account) {
        untracked(_controller.visit);
      }
    });
  }

  @override
  void dispose() {
    _stopVisits();
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    body: SafeArea(
      child: SignalBuilder(
        builder: (context) {
          if (!_controller.opened.value) return const SizedBox.shrink();
          return LayoutBuilder(
            builder: (context, constraints) {
              final width = constraints.maxWidth;
              if (width < _twoPaneWidth) {
                return _Stacked(controller: _controller);
              }
              return AdaptivePanes(
                primaryFraction: (_sideNavWidth / width).clamp(0.2, 0.42),
                minWidth: _twoPaneWidth,
                primary: _SideNav(controller: _controller),
                secondary: _SectionPane(controller: _controller),
              );
            },
          );
        },
      ),
    ),
  );
}

class _Stacked extends StatelessWidget {
  const _Stacked({required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return CustomScrollView(
      slivers: [
        SliverPadding(
          padding: EdgeInsets.fromLTRB(gutter, 20, gutter, 10),
          sliver: const SliverToBoxAdapter(child: _Header(stacked: false)),
        ),
        SliverToBoxAdapter(child: _SectionChips(controller: controller)),
        SliverPadding(
          padding: EdgeInsets.fromLTRB(gutter, 8, gutter, 32),
          sliver: _SectionSliver(controller: controller),
        ),
      ],
    );
  }
}

class _SideNav extends StatelessWidget {
  const _SideNav({required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) => DecoratedBox(
    decoration: const BoxDecoration(
      border: Border(right: BorderSide(color: NfColors.line)),
    ),
    child: ListView(
      padding: const EdgeInsets.fromLTRB(16, 20, 12, 24),
      children: [
        const _Header(stacked: true),
        const SizedBox(height: 20),
        SignalBuilder(
          builder: (context) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final section in AccountSection.values)
                _SideNavItem(
                  section: section,
                  selected: controller.section.value == section,
                  count: _countOf(controller, section),
                  onTap: () => controller.select(section),
                ),
            ],
          ),
        ),
      ],
    ),
  );
}

/// What's worth noticing from outside a section.
int? _countOf(AccountController controller, AccountSection section) =>
    switch (section) {
      AccountSection.devices =>
        controller.sessions.value.isEmpty
            ? null
            : controller.sessions.value.length,
      AccountSection.exports =>
        controller.hasActiveExports.value
            ? controller.exportJobs.value.where(isActiveExport).length
            : null,
      _ => null,
    };

class _SideNavItem extends StatelessWidget {
  const _SideNavItem({
    required this.section,
    required this.selected,
    required this.count,
    required this.onTap,
  });

  final AccountSection section;
  final bool selected;
  final int? count;
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
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 10),
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
              ],
            ),
          ),
        ),
      ),
    ),
  );
}

class _SectionPane extends StatelessWidget {
  const _SectionPane({required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return SignalBuilder(
      builder: (context) => CustomScrollView(
        // A fresh position per section: one's scroll offset means nothing to
        // the next.
        key: ValueKey(controller.section.value),
        slivers: [
          SliverPadding(
            padding: EdgeInsets.fromLTRB(gutter, 24, gutter, 32),
            sliver: _SectionSliver(controller: controller),
          ),
        ],
      ),
    );
  }
}

class _SectionChips extends SignalWidget {
  const _SectionChips({required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return SizedBox(
      height: 52,
      child: ListView(
        scrollDirection: Axis.horizontal,
        padding: EdgeInsets.fromLTRB(gutter, 6, gutter, 8),
        children: [
          for (final section in AccountSection.values)
            Padding(
              padding: const EdgeInsets.only(right: 8),
              child: NfChoiceChip(
                label: section.label,
                detail: _countOf(controller, section)?.toString(),
                selected: controller.section.value == section,
                onTap: () => controller.select(section),
              ),
            ),
        ],
      ),
    );
  }
}

class _SectionSliver extends StatelessWidget {
  const _SectionSliver({required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final section = controller.section.value;
      switch (section) {
        case AccountSection.history:
          return HistorySection(controller: controller);
        case AccountSection.servers:
          return SliverToBoxAdapter(
            child: ServersSection(controller: controller),
          );
        case AccountSection.profile ||
            AccountSection.devices ||
            AccountSection.exports:
          break;
      }
      final query = controller.query;
      if (query.waiting.value) {
        return const SliverFillRemaining(
          hasScrollBody: false,
          child: Center(child: CircularProgressIndicator()),
        );
      }
      if (query.data.value == null) {
        return SliverFillRemaining(
          hasScrollBody: false,
          child: NfState.error(
            title: 'Your account didn’t load',
            message: query.error.value?.toString(),
            retry: query.refetch,
          ),
        );
      }
      return SliverToBoxAdapter(
        child: switch (section) {
          AccountSection.devices => DevicesSection(controller: controller),
          AccountSection.exports => ExportsSection(controller: controller),
          _ => ProfileSection(controller: controller),
        },
      );
    },
  );
}

class _Header extends SignalWidget {
  const _Header({required this.stacked});

  /// In a narrow pane, where Sign out goes below the name.
  final bool stacked;

  @override
  Widget build(BuildContext context) {
    final session = getIt<SessionService>();
    final viewer = session.viewer.value;
    if (viewer == null) return const SizedBox.shrink();
    final name = viewer.name;
    final initials = name.length > 2 ? name.substring(0, 2) : name;
    final signOutButton = NfButton(
      label: 'Sign out',
      style: NfButtonStyle.ghost,
      size: NfButtonSize.small,
      icon: const Icon(PhosphorIconsRegular.signOut),
      onPressed: () => unawaited(signOut()),
    );
    final identity = Row(
      children: [
        Container(
          width: 52,
          height: 52,
          alignment: Alignment.center,
          decoration: const BoxDecoration(
            color: NfColors.accent,
            shape: BoxShape.circle,
          ),
          child: Text(
            initials.toUpperCase(),
            style: nfText(19, weight: 700, color: NfColors.accentInk),
          ),
        ),
        const SizedBox(width: 14),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: nfText(21, weight: 650, letterSpacing: -0.4),
              ),
              const SizedBox(height: 2),
              Text(
                '${titleCase(viewer.role.name)} · Member since ${monthYear(viewer.createdAt)}',
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: NfType.caption,
              ),
            ],
          ),
        ),
      ],
    );
    if (stacked) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [identity, const SizedBox(height: 12), signOutButton],
      );
    }
    return Row(
      children: [
        Expanded(child: identity),
        const SizedBox(width: 8),
        signOutButton,
      ],
    );
  }
}
