// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/models/work.dart';
import 'package:noflex/utils/images.dart';
import 'package:noflex/widgets/poster_flight.dart';

void main() {
  const work = WorkSummary(id: 'w', kind: 'MOVIE', libraryId: 'l', title: 'W');

  Widget poster(Object tag, double width, double radius) => SizedBox(
    width: width,
    height: width * 3 / 2,
    child: PosterHero(
      tag: tag,
      work: work,
      kind: ArtKind.poster,
      radius: radius,
    ),
  );

  testWidgets('one of several cards for a title flies, at its own size', (
    tester,
  ) async {
    final tapped = Object();
    final other = Object();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Column(
            children: [poster(tapped, 120, 13), poster(other, 120, 13)],
          ),
        ),
      ),
    );

    final navigator = tester.state<NavigatorState>(find.byType(Navigator));
    final done = navigator.push(
      MaterialPageRoute<void>(
        builder: (_) => Scaffold(body: poster(tapped, 300, 18)),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));

    final shuttle = find
        .descendant(of: find.byType(FittedBox), matching: find.byType(SizedBox))
        .first;
    expect(tester.getSize(find.byType(FittedBox)).width, greaterThan(120));
    expect(tester.widget<SizedBox>(shuttle).width, 120);

    await tester.pumpAndSettle();
    expect(find.byType(FittedBox), findsNothing);

    navigator.pop();
    await done;
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    expect(tester.widget<SizedBox>(shuttle).width, 120);
    await tester.pumpAndSettle();
  });
}
