// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/layout/window.dart';

Fold verticalFold(double x) =>
    Fold(Rect.fromLTWH(x, 0, 0, 800), halfOpen: false);

void main() {
  group('splitAroundFold', () {
    test('splits on a fold through the middle of the panes', () {
      // A Pixel Fold's crease at 420, with a 76 wide rail left of the panes.
      final widths = splitAroundFold(
        totalWidth: 765,
        origin: 76,
        fold: verticalFold(420),
        primaryFraction: 0.42,
        gap: 0,
      );
      expect(widths.primary, 344);
    });

    test('keeps a hinge with width clear of both panes', () {
      final widths = splitAroundFold(
        totalWidth: 800,
        origin: 0,
        fold: Fold(const Rect.fromLTWH(400, 0, 28, 800), halfOpen: true),
        primaryFraction: 0.42,
        gap: 0,
      );
      expect(widths.primary, 400);
      expect(widths.gap, 28);
    });

    test('ignores a fold near an edge rather than squeezing a pane', () {
      final widths = splitAroundFold(
        totalWidth: 1000,
        origin: 0,
        fold: verticalFold(60),
        primaryFraction: 0.4,
        gap: 0,
      );
      expect(widths.primary, 400);
    });

    test('a horizontal fold changes nothing side to side', () {
      final widths = splitAroundFold(
        totalWidth: 1000,
        origin: 0,
        fold: Fold(const Rect.fromLTWH(0, 400, 1000, 0), halfOpen: true),
        primaryFraction: 0.4,
        gap: 0,
      );
      expect(widths.primary, 400);
    });
  });

  testWidgets(
    'AdaptivePanes stacks on a phone and goes side by side when unfolded',
    (tester) async {
      Future<void> pump(Size size) async {
        tester.view.physicalSize = size;
        tester.view.devicePixelRatio = 1;
        await tester.pumpWidget(
          const MaterialApp(
            home: AdaptivePanes(
              primary: Text('primary'),
              secondary: Text('secondary'),
            ),
          ),
        );
      }

      addTearDown(tester.view.reset);
      await pump(const Size(412, 900));
      expect(find.byType(CustomScrollView), findsOneWidget);
      await pump(const Size(841, 700));
      expect(find.byType(CustomScrollView), findsNothing);
      expect(find.byType(Row), findsOneWidget);
    },
  );

  test('posterColumns never drops below two and grows with the window', () {
    expect(posterColumns(320), 2);
    expect(posterColumns(841), greaterThan(posterColumns(412)));
  });
}
