// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:built_collection/built_collection.dart';
import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/search.req.gql.dart';
import '../../graphql/query.dart';
import '../../layout/window.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../utils/merge.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_state.dart';
import '../../widgets/work_grid.dart';

const _modes = [
  (GSearchMode.HYBRID, 'Balanced'),
  (GSearchMode.LEXICAL, 'Exact words'),
  (GSearchMode.VECTOR, 'Meaning'),
];

const _examples = [
  'a rainy detective story',
  'heist gone wrong',
  'feel-good 90s comedy',
  'space survival',
  'jazz for a late night',
];

class SearchScreen extends StatefulWidget {
  const SearchScreen({super.key});

  @override
  State<SearchScreen> createState() => _SearchScreenState();
}

class _SearchScreenState extends State<SearchScreen>
    with AutomaticKeepAliveClientMixin {
  final _controller = TextEditingController();
  final _text = signal('');
  final _mode = signal(GSearchMode.HYBRID);
  Timer? _debounce;
  late final _graphql = getIt<GraphqlService>();

  late final Pager<WorkSummary> _pager = Pager(_page);

  /// This server pages by cursor; a linked server's first page joins the first
  /// page here, merged by rank, since its deadline must not hold up scrolling.
  Future<(List<WorkSummary>, String?)> _page(String? after) async {
    final text = _text.value.trim();
    Future<List<WorkSummary>> elsewhere(ServerRef server) async {
      try {
        final data = await _graphql.run(
          GSearchReq(
            (b) => b
              ..fetchPolicy = FetchPolicy.NetworkOnly
              ..vars.text = text
              ..vars.mode = _mode.value,
          ),
          server: server,
          timeout: const Duration(seconds: 20),
        );
        return [
          for (final hit in data.search.nodes)
            if (hit.work case final work?)
              WorkSummary.fromCard(
                work,
                overview: work.overview,
                seriesTitle: work.seriesTitle,
                server: server,
              ),
        ];
      } on Exception {
        return const [];
      }
    }

    final others = after == null
        ? Future.wait(
            getIt<SessionService>().connectedServers.value.map(elsewhere),
          )
        : Future.value(const <List<WorkSummary>>[]);
    final data = await _graphql.run(
      GSearchReq(
        (b) => b
          ..fetchPolicy = FetchPolicy.NetworkOnly
          ..vars.text = text
          ..vars.mode = _mode.value
          ..vars.after = after
          ..vars.kinds = ListBuilder<GWorkKind>(),
      ).rebuild((b) => b.vars.kinds = null),
    );
    final own = [
      for (final hit in data.search.nodes)
        if (hit.work case final work?)
          WorkSummary.fromCard(
            work,
            overview: work.overview,
            seriesTitle: work.seriesTitle,
            seriesExternalIds: [
              for (final id in work.seriesExternalIds)
                ExternalId(id.kind.name, id.value),
            ],
          ),
    ];
    final info = data.search.pageInfo;
    final merged = mergeRanked([own, ...await others]);
    return (merged, info.hasNextPage ? info.endCursor : null);
  }

  void _onChanged(String value) {
    _text.value = value;
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 320), () {
      if (value.trim().length >= 2) unawaited(_pager.refresh());
    });
  }

  void _useExample(String example) {
    _controller.text = example;
    _onChanged(example);
  }

  @override
  void dispose() {
    _debounce?.cancel();
    _controller.dispose();
    super.dispose();
  }

  @override
  bool get wantKeepAlive => true;

  @override
  Widget build(BuildContext context) {
    super.build(context);
    final gutter = context.gutter;
    return SafeArea(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: EdgeInsets.fromLTRB(gutter, 14, gutter, 10),
            child: NfField(
              controller: _controller,
              hint: 'Titles, people, or what it’s about',
              textInputAction: TextInputAction.search,
              onChanged: _onChanged,
              prefixIcon: const Icon(
                PhosphorIconsRegular.magnifyingGlass,
                size: 18,
                color: NfColors.text3,
              ),
              suffix: SignalBuilder(
                builder: (_) => _text.value.isEmpty
                    ? const SizedBox.shrink()
                    : IconButton(
                        tooltip: 'Clear',
                        icon: const Icon(
                          PhosphorIconsRegular.x,
                          size: 16,
                          color: NfColors.text2,
                        ),
                        onPressed: () {
                          _controller.clear();
                          _text.value = '';
                        },
                      ),
              ),
            ),
          ),
          SizedBox(
            height: 38,
            child: SignalBuilder(
              builder: (_) => ListView(
                scrollDirection: Axis.horizontal,
                padding: EdgeInsets.symmetric(horizontal: gutter),
                children: [
                  for (final (mode, label) in _modes)
                    Padding(
                      padding: const EdgeInsets.only(right: 8),
                      child: NfChoiceChip(
                        label: label,
                        selected: _mode.value == mode,
                        onTap: () {
                          _mode.value = mode;
                          if (_text.value.trim().length >= 2) {
                            unawaited(_pager.refresh());
                          }
                        },
                      ),
                    ),
                ],
              ),
            ),
          ),
          Expanded(
            child: SignalBuilder(
              builder: (_) {
                if (_text.value.trim().length < 2) {
                  return NfState(
                    icon: PhosphorIconsRegular.sparkle,
                    title: 'Search by meaning, not just words',
                    message: 'Describe what you’re in the mood for.',
                    child: Padding(
                      padding: const EdgeInsets.only(top: 16),
                      child: Wrap(
                        alignment: WrapAlignment.center,
                        spacing: 8,
                        runSpacing: 8,
                        children: [
                          for (final example in _examples)
                            NfChoiceChip(
                              label: example,
                              selected: false,
                              onTap: () => _useExample(example),
                            ),
                        ],
                      ),
                    ),
                  );
                }
                return CustomScrollView(
                  slivers: [
                    SliverWorkGrid(
                      pager: _pager,
                      emptyTitle: 'No matches',
                      emptyMessage:
                          'Try different words, or switch how search matches.',
                    ),
                  ],
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}
