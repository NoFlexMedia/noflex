// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/services/nav_service.dart';

void main() {
  group('the place the navigation shows', () {
    late NavService nav;

    Future<void> start(WidgetTester tester) async {
      nav = NavService();
      await tester.pumpWidget(
        MaterialApp(
          navigatorKey: rootNavigatorKey,
          home: Navigator(
            key: contentNavigatorKey,
            observers: [nav.placeObserver],
            onGenerateRoute: (settings) => MaterialPageRoute<void>(
              settings: settings,
              builder: (_) => const Text('tab'),
            ),
          ),
        ),
      );
    }

    testWidgets('stays lit under a page opened from it, until it’s left', (
      tester,
    ) async {
      await start(tester);
      nav.open('movies', const Text('movies'));
      await tester.pumpAndSettle();
      expect(nav.place.value, 'movies');

      nav.push(const Text('a film')).ignore();
      await tester.pumpAndSettle();
      expect(nav.place.value, 'movies');

      contentNavigatorKey.currentState!.pop();
      await tester.pumpAndSettle();
      expect(nav.place.value, 'movies');

      contentNavigatorKey.currentState!.pop();
      await tester.pumpAndSettle();
      expect(nav.place.value, isNull);
    });

    testWidgets('going elsewhere replaces it, so back leaves for the tab', (
      tester,
    ) async {
      await start(tester);
      nav.open('movies', const Text('movies'));
      await tester.pumpAndSettle();
      nav.open('shows', const Text('shows'));
      await tester.pumpAndSettle();
      expect(nav.place.value, 'shows');
      expect(find.text('shows'), findsOneWidget);

      contentNavigatorKey.currentState!.pop();
      await tester.pumpAndSettle();
      expect(nav.place.value, isNull);
      expect(find.text('tab'), findsOneWidget);
    });

    testWidgets('going where you are returns to its top, unless asked afresh', (
      tester,
    ) async {
      await start(tester);
      nav.open('admin', const Text('first'));
      await tester.pumpAndSettle();
      nav.push(const Text('a film')).ignore();
      await tester.pumpAndSettle();

      nav.open('admin', const Text('second'));
      await tester.pumpAndSettle();
      expect(find.text('first'), findsOneWidget);
      expect(find.text('a film'), findsNothing);

      nav.open('admin', const Text('second'), fresh: true);
      await tester.pumpAndSettle();
      expect(find.text('second'), findsOneWidget);
      expect(find.text('first'), findsNothing);
    });

    testWidgets('a new navigator doesn’t inherit the old one’s place', (
      tester,
    ) async {
      await start(tester);
      nav.open('admin', const Text('admin'));
      await tester.pumpAndSettle();

      await tester.pumpWidget(const SizedBox());
      await tester.pumpWidget(
        MaterialApp(
          home: Navigator(
            key: contentNavigatorKey,
            observers: [nav.placeObserver],
            onGenerateRoute: (settings) => MaterialPageRoute<void>(
              settings: settings,
              builder: (_) => const Text('tab'),
            ),
          ),
        ),
      );
      await tester.pump();
      expect(nav.place.value, isNull);
    });
  });
}
