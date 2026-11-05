// SPDX-License-Identifier: AGPL-3.0-only

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/library_works.req.gql.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import 'work_browser.dart';

/// Every top-level work in one library, for those who browse by folder.
class LibraryScreen extends StatelessWidget {
  const LibraryScreen({
    super.key,
    required this.libraryId,
    required this.title,
  });

  final String libraryId;
  final String title;

  Future<(List<WorkSummary>, String?)> _page(
    String? after,
    GWorkFilterInput? filter,
    GSortOrder sort,
  ) async {
    final data = await getIt<GraphqlService>().run(
      GLibraryWorksReq(
        (b) => b
          ..fetchPolicy = FetchPolicy.NetworkOnly
          ..vars.library = libraryId
          ..vars.sort = sort
          ..vars.after = after
          ..vars.filter = (filter ?? GWorkFilterInput())
              .rebuild((f) => f.recursive = false)
              .toBuilder(),
      ),
    );
    final info = data.works.pageInfo;
    return (
      [for (final work in data.works.nodes) WorkSummary.fromCard(work)],
      info.hasNextPage ? info.endCursor : null,
    );
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: NfPageBar(title: title),
    body: WorkBrowser(page: _page),
  );
}
