// SPDX-License-Identifier: AGPL-3.0-only
//
// A filterable grid over one category or library. The web's full rule editor
// stays on the web; this is the sort and the watched state you'd reach for on a
// couch.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/query.dart';
import '../../layout/window.dart';
import '../../models/work.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/work_grid.dart';

enum WatchFilter { all, unwatched, watched, favorites }

const sortLabels = {
  GSortOrder.DATE_ADDED_DESC: 'Recently added',
  GSortOrder.TITLE_ASC: 'Title A–Z',
  GSortOrder.YEAR_DESC: 'Newest',
  GSortOrder.YEAR_ASC: 'Oldest',
  GSortOrder.DATE_RELEASED_DESC: 'Recently released',
  GSortOrder.RANDOM: 'Shuffle',
};

GWorkFilterInput? buildFilter(WatchFilter filter) {
  final user = getIt<SessionService>().viewer.value?.id;
  if (user == null) return null;
  return switch (filter) {
    WatchFilter.all => null,
    WatchFilter.favorites => GWorkFilterInput((b) => b.favoriteFor = user),
    WatchFilter.watched || WatchFilter.unwatched => GWorkFilterInput(
      (b) => b.watched
        ..userId = user
        ..watched = filter == WatchFilter.watched,
    ),
  };
}

typedef WorkPage = Future<(List<WorkSummary>, String?)> Function(
  String? after,
  GWorkFilterInput? filter,
  GSortOrder sort,
);

class WorkBrowser extends StatefulWidget {
  const WorkBrowser({super.key, required this.page, this.header});

  final WorkPage page;
  final Widget? header;

  @override
  State<WorkBrowser> createState() => _WorkBrowserState();
}

class _WorkBrowserState extends State<WorkBrowser> {
  final _sort = signal(GSortOrder.DATE_ADDED_DESC);
  final _filter = signal(WatchFilter.all);
  late final _pager = Pager<WorkSummary>(
    (after) => widget.page(after, buildFilter(_filter.value), _sort.value),
  );

  @override
  void initState() {
    super.initState();
    unawaited(_pager.refresh());
  }

  Future<void> _chooseSort() async {
    final choice = await pickOption(
      title: 'Sort by',
      selected: _sort.value,
      options: [
        for (final entry in sortLabels.entries)
          NfOption(entry.key, entry.value),
      ],
    );
    if (choice == null) return;
    _sort.value = choice;
    unawaited(_pager.refresh());
  }

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return RefreshIndicator(
      onRefresh: _pager.refresh,
      child: CustomScrollView(
        slivers: [
          ?widget.header,
          SliverToBoxAdapter(
            child: SizedBox(
              height: 52,
              child: SignalBuilder(
                builder: (_) => ListView(
                  scrollDirection: Axis.horizontal,
                  padding: EdgeInsets.fromLTRB(gutter, 6, gutter, 8),
                  children: [
                    NfChoiceChip(
                      label: sortLabels[_sort.value] ?? 'Sort',
                      selected: false,
                      leading: const Icon(PhosphorIconsRegular.sortAscending),
                      onTap: _chooseSort,
                    ),
                    const SizedBox(width: 8),
                    for (final (filter, label) in const [
                      (WatchFilter.all, 'All'),
                      (WatchFilter.unwatched, 'Unwatched'),
                      (WatchFilter.watched, 'Watched'),
                      (WatchFilter.favorites, 'Favorites'),
                    ])
                      Padding(
                        padding: const EdgeInsets.only(right: 8),
                        child: NfChoiceChip(
                          label: label,
                          selected: _filter.value == filter,
                          onTap: () {
                            _filter.value = filter;
                            unawaited(_pager.refresh());
                          },
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
          SliverWorkGrid(
            pager: _pager,
            emptyTitle: 'Nothing matches',
            emptyMessage: 'Try another filter.',
          ),
        ],
      ),
    );
  }
}

/// A plain title bar for pages pushed from the shell.
class NfPageBar extends StatelessWidget implements PreferredSizeWidget {
  const NfPageBar({super.key, required this.title, this.actions});

  final String title;
  final List<Widget>? actions;

  @override
  Size get preferredSize => const Size.fromHeight(56);

  @override
  Widget build(BuildContext context) => AppBar(
    backgroundColor: NfColors.bg,
    surfaceTintColor: Colors.transparent,
    foregroundColor: NfColors.text,
    title: Text(title, style: nfText(17, weight: 650, letterSpacing: -0.3)),
    actions: actions,
  );
}
