// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:ui' show DisplayFeature, DisplayFeatureState, DisplayFeatureType;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/layout/window.dart';

Fold verticalFold(double x) =>
    Fold(Rect.fromLTWH(x, 0, 0, 800), halfOpen: false);

void main() {
  group('splitAroundFold', () {
    test('splits on a fold through the middle of the panes', () {
      final widths = splitAroundFold(
        totalWidth: 765,
        fold: verticalFold(344),
        primaryFraction: 0.42,
        gap: 0,
      );
      expect(widths.primary, 344);
    });

    test('keeps a hinge with width clear of both panes', () {
      final widths = splitAroundFold(
        totalWidth: 800,
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
        fold: verticalFold(60),
        primaryFraction: 0.4,
        gap: 0,
      );
      expect(widths.primary, 400);
    });

    test('a horizontal fold changes nothing side to side', () {
      final widths = splitAroundFold(
        totalWidth: 1000,
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

  testWidgets('a list sized to itself ignores a flat fold, not a bent one', (
    tester,
  ) async {
    Future<double> listWidth({required bool halfOpen}) async {
      await tester.pumpWidget(
        MaterialApp(
          home: MediaQuery(
            data: MediaQueryData(
              size: const Size(765, 800),
              displayFeatures: [
                DisplayFeature(
                  bounds: const Rect.fromLTWH(344, 0, 0, 800),
                  type: DisplayFeatureType.fold,
                  state: halfOpen
                      ? DisplayFeatureState.postureHalfOpened
                      : DisplayFeatureState.postureFlat,
                ),
              ],
            ),
            child: const AdaptivePanes(
              primaryFits: true,
              splitOnFlatFold: false,
              minWidth: 700,
              primary: SizedBox(key: ValueKey('list'), width: 190),
              secondary: SizedBox.expand(),
            ),
          ),
        ),
      );
      return tester.getSize(find.byKey(const ValueKey('list'))).width;
    }

    tester.view.physicalSize = const Size(765, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    expect(await listWidth(halfOpen: false), 190);
    expect(await listWidth(halfOpen: true), 344);
  });

  testWidgets('AdaptivePanes splits on the fold from its first frame', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(841, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    const primary = Key('primary');
    // A Pixel Fold's crease at 420, with a 76 wide rail beside the panes.
    await tester.pumpWidget(
      const Directionality(
        textDirection: TextDirection.ltr,
        child: MediaQuery(
          data: MediaQueryData(
            displayFeatures: [
              DisplayFeature(
                bounds: Rect.fromLTWH(420, 0, 0, 700),
                type: DisplayFeatureType.fold,
                state: DisplayFeatureState.postureFlat,
              ),
            ],
          ),
          child: Row(
            children: [
              SizedBox(width: 76),
              Expanded(
                child: FoldsFromContent(
                  left: 76,
                  child: AdaptivePanes(
                    primary: SizedBox(key: primary),
                    secondary: SizedBox(),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
    expect(tester.getSize(find.byKey(primary)).width, 344);
  });

  test('posterColumns never drops below two and grows with the window', () {
    expect(posterColumns(320), 2);
    expect(posterColumns(841), greaterThan(posterColumns(412)));
  });
}
