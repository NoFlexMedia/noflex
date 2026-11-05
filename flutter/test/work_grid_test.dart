// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/di.dart';
import 'package:noflex/features/player/playback_service.dart';
import 'package:noflex/graphql/query.dart';
import 'package:noflex/models/work.dart';
import 'package:noflex/services/watch_state.dart';
import 'package:noflex/widgets/work_grid.dart';

void main() {
  setUp(() {
    getIt
      ..registerSingleton(WatchStateService())
      ..registerSingleton(PlaybackService());
  });
  tearDown(getIt.reset);

  testWidgets('a failed later page waits to be asked again', (tester) async {
    var asked = 0;
    final pager = Pager<WorkSummary>((after) async {
      asked++;
      if (after != null) throw Exception('gone');
      return (
        [
          for (var i = 0; i < 6; i++)
            WorkSummary(id: '$i', kind: 'MOVIE', libraryId: 'l', title: '$i'),
        ],
        'next',
      );
    });
    await pager.refresh();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: CustomScrollView(slivers: [SliverWorkGrid(pager: pager)]),
        ),
      ),
    );
    for (var frame = 0; frame < 10; frame++) {
      await tester.pump();
    }
    expect(asked, 2);

    await tester.ensureVisible(find.text('Load more'));
    await tester.tap(find.text('Load more'));
    for (var frame = 0; frame < 10; frame++) {
      await tester.pump();
    }
    expect(asked, 3);
  });
}
