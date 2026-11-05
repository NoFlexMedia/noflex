// SPDX-License-Identifier: AGPL-3.0-only

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/category_shelves.data.gql.dart';
import '../../graphql/queries/__generated__/category_shelves.req.gql.dart';
import '../../graphql/queries/__generated__/category_shelves.var.gql.dart';
import '../../graphql/queries/__generated__/category_works.req.gql.dart';
import '../../graphql/query.dart';
import '../../layout/window.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../utils/categories.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_state.dart';
import '../../widgets/rail.dart';
import 'work_browser.dart';

class CategoryScreen extends StatefulWidget {
  const CategoryScreen({super.key, required this.category});
  final Category category;

  @override
  State<CategoryScreen> createState() => _CategoryScreenState();
}

class _CategoryScreenState extends State<CategoryScreen> {
  late final _showAll = signal(!widget.category.shelves);
  late final Query<GCategoryShelvesData, GCategoryShelvesVars>? _shelves =
      widget.category.shelves
      ? Query(
          getIt<GraphqlService>().client,
          GCategoryShelvesReq((b) => b.vars.category = widget.category.value),
        )
      : null;

  Future<(List<WorkSummary>, String?)> _page(
    String? after,
    GWorkFilterInput? filter,
    GSortOrder sort,
  ) async {
    final data = await getIt<GraphqlService>().run(
      GCategoryWorksReq(
        (b) => b
          ..fetchPolicy = FetchPolicy.NetworkOnly
          ..vars.category = widget.category.value
          ..vars.sort = sort
          ..vars.after = after
          ..vars.filter = filter?.toBuilder(),
      ),
    );
    final info = data.works.pageInfo;
    return (
      [for (final work in data.works.nodes) WorkSummary.fromCard(work)],
      info.hasNextPage ? info.endCursor : null,
    );
  }

  @override
  void dispose() {
    _shelves?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: NfPageBar(title: widget.category.label),
    body: SignalBuilder(
      builder: (_) {
        final all = _showAll.value;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (widget.category.shelves)
              Padding(
                padding: EdgeInsets.fromLTRB(
                  context.gutter,
                  4,
                  context.gutter,
                  4,
                ),
                child: Row(
                  children: [
                    NfChoiceChip(
                      label: 'For you',
                      selected: !all,
                      onTap: () => _showAll.value = false,
                    ),
                    const SizedBox(width: 8),
                    NfChoiceChip(
                      label: 'All',
                      selected: all,
                      onTap: () => _showAll.value = true,
                    ),
                  ],
                ),
              ),
            Expanded(
              child: all
                  ? WorkBrowser(page: _page)
                  : _Shelves(
                      query: _shelves!,
                      onShowAll: () => _showAll.value = true,
                    ),
            ),
          ],
        );
      },
    ),
  );
}

class _Shelves extends SignalWidget {
  const _Shelves({required this.query, required this.onShowAll});

  final Query<GCategoryShelvesData, GCategoryShelvesVars> query;
  final VoidCallback onShowAll;

  @override
  Widget build(BuildContext context) {
    final data = query.data.value;
    if (query.waiting.value) {
      return ListView(children: const [RailSkeleton(), RailSkeleton()]);
    }
    if (data == null) {
      return NfState.error(
        title: 'This didn’t load',
        message: query.error.value?.toString(),
        retry: query.refetch,
      );
    }
    final recommended = [
      for (final w in data.recommendations.nodes) WorkSummary.fromCard(w),
    ];
    final released = [
      for (final w in data.newlyReleased.nodes) WorkSummary.fromCard(w),
    ];
    final added = [
      for (final w in data.recentlyAdded.nodes) WorkSummary.fromCard(w),
    ];
    return ListView(
      padding: const EdgeInsets.only(bottom: 32),
      children: [
        if (recommended.isNotEmpty)
          Rail(title: 'Recommended', items: recommended),
        if (released.isNotEmpty) Rail(title: 'New releases', items: released),
        if (added.isNotEmpty)
          Rail(title: 'Recently added', items: added, onHeader: onShowAll),
      ],
    );
  }
}
