// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../graphql/query.dart';
import '../layout/window.dart';
import '../models/work.dart';
import '../theme.dart';
import 'nf_button.dart';
import 'nf_state.dart';
import 'work_card.dart';

/// A paged poster grid as a sliver, loading more as it nears the end.
class SliverWorkGrid extends StatelessWidget {
  const SliverWorkGrid({
    super.key,
    required this.pager,
    this.emptyTitle = 'Nothing here yet',
    this.emptyMessage,
  });

  final Pager<WorkSummary> pager;
  final String emptyTitle;
  final String? emptyMessage;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return SignalBuilder(
      builder: (context) {
        final items = pager.items.value;
        if (pager.waiting.value ||
            (items.isEmpty &&
                pager.hasMore.value &&
                pager.error.value == null)) {
          return SliverPadding(
            padding: EdgeInsets.fromLTRB(gutter, 8, gutter, 32),
            sliver: SliverLayoutBuilder(
              builder: (context, constraints) {
                final columns = posterColumns(constraints.crossAxisExtent);
                return SliverGrid.builder(
                  gridDelegate: _gridDelegate(
                    columns,
                    constraints.crossAxisExtent,
                  ),
                  itemCount: columns * 3,
                  itemBuilder: (_, _) => const Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Expanded(child: NfSkeleton()),
                      SizedBox(height: 10),
                      NfSkeleton(height: 12, width: 96, radius: NfRadii.xs),
                      SizedBox(height: 8),
                      NfSkeleton(height: 10, width: 60, radius: NfRadii.xs),
                      SizedBox(height: 6),
                    ],
                  ),
                );
              },
            ),
          );
        }
        final failure = pager.error.value;
        if (failure != null && items.isEmpty) {
          return SliverFillRemaining(
            hasScrollBody: false,
            child: NfState.error(
              title: 'This didn’t load',
              message: failure.toString(),
              retry: pager.refresh,
            ),
          );
        }
        if (items.isEmpty) {
          return SliverFillRemaining(
            hasScrollBody: false,
            child: NfState(title: emptyTitle, message: emptyMessage),
          );
        }
        return SliverPadding(
          padding: EdgeInsets.fromLTRB(gutter, 8, gutter, 32),
          sliver: SliverMainAxisGroup(
            slivers: [
              SliverLayoutBuilder(
                builder: (context, constraints) {
                  final columns = posterColumns(constraints.crossAxisExtent);
                  return SliverGrid.builder(
                    gridDelegate: _gridDelegate(
                      columns,
                      constraints.crossAxisExtent,
                    ),
                    itemCount: items.length,
                    itemBuilder: (context, index) {
                      // Failing rebuilds this, so asking again from here would
                      // retry for as long as the failure lasts. The button does.
                      if (failure == null &&
                          index >= items.length - columns * 3 &&
                          pager.hasMore.value) {
                        WidgetsBinding.instance.addPostFrameCallback(
                          (_) => pager.loadMore(),
                        );
                      }
                      return WorkCard(work: items[index]);
                    },
                  );
                },
              ),
              if (failure != null)
                SliverToBoxAdapter(
                  child: Padding(
                    padding: const EdgeInsets.only(top: 20),
                    child: Center(
                      child: NfButton(
                        label: 'Load more',
                        onPressed: pager.loadMore,
                      ),
                    ),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }
}

SliverGridDelegate _gridDelegate(int columns, double width) {
  final cell = (width - (columns - 1) * 12) / columns;
  return SliverGridDelegateWithFixedCrossAxisCount(
    crossAxisCount: columns,
    crossAxisSpacing: 12,
    mainAxisSpacing: 16,
    mainAxisExtent: cell * 3 / 2 + 46,
  );
}
