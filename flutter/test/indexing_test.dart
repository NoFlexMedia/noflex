// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/di.dart';
import 'package:noflex/models/work.dart';
import 'package:noflex/services/indexing_service.dart';
import 'package:noflex/widgets/indexing_overlay.dart';

const _unmatched = WorkSummary(
  id: 'w',
  kind: 'EPISODE',
  libraryId: 'l',
  title: 'episode.s01e01.1080p',
  seriesTitle: 'The Show',
  indexing: 'FINDING_DETAILS',
);

void main() {
  setUp(() => getIt.registerSingleton(IndexingService()));
  tearDown(getIt.reset);

  test('an answer brings the match and keeps what only the list knew', () {
    const answer = WorkSummary(
      id: 'w',
      kind: 'EPISODE',
      libraryId: 'l',
      title: 'Pilot',
    );
    final updated = _unmatched.updatedBy(answer);
    expect(updated.title, 'Pilot');
    expect(updated.seriesTitle, 'The Show');
    expect(updated.indexing, isNull);
  });

  test('a work that has gone stops indexing rather than waiting forever', () {
    final updated = _unmatched.updatedBy(null);
    expect(updated.title, _unmatched.title);
    expect(updated.indexing, isNull);
  });

  test('a linked server\'s title is left as its list had it', () {
    const linked = WorkSummary(
      id: 'w',
      kind: 'MOVIE',
      libraryId: 'l',
      title: 'Film',
      indexing: 'ADDING',
      server: ServerRef(id: 's', name: 'Elsewhere', origin: 'o', apiKey: 'k'),
    );
    expect(getIt<IndexingService>().latest(linked).indexing, 'ADDING');
  });

  // Cards come in every size from a list row's thumbnail up, and a layout that
  // doesn't fit fails the pump.
  for (final (:width, :height, :wide, :hasArt) in const [
    (width: 56.0, height: 84.0, wide: false, hasArt: false),
    (width: 110.0, height: 165.0, wide: false, hasArt: false),
    (width: 190.0, height: 285.0, wide: false, hasArt: false),
    (width: 110.0, height: 165.0, wide: false, hasArt: true),
    (width: 130.0, height: 73.0, wide: true, hasArt: false),
    (width: 340.0, height: 191.0, wide: true, hasArt: false),
    (width: 340.0, height: 191.0, wide: true, hasArt: true),
  ]) {
    testWidgets('fits ${width}x$height wide=$wide art=$hasArt', (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          home: Center(
            child: SizedBox(
              width: width,
              height: height,
              child: IndexingOverlay(
                workId: 'w',
                stage: 'FINDING_DETAILS',
                hasArt: hasArt,
                wide: wide,
              ),
            ),
          ),
        ),
      );
      await tester.pump(const Duration(milliseconds: 800));
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets('holds still when animations are off', (tester) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: MediaQuery(
          data: MediaQueryData(disableAnimations: true),
          child: IndexingOverlay(
            workId: 'w',
            stage: 'ADDING',
            hasArt: false,
            wide: false,
          ),
        ),
      ),
    );
    expect(tester.hasRunningAnimations, isFalse);
    expect(find.text('Adding to library'), findsOneWidget);
  });
}
