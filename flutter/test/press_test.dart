// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/widgets/nf_button.dart';
import 'package:noflex/widgets/nf_press.dart';

double _targetScale(WidgetTester tester) =>
    tester.widget<AnimatedScale>(find.byType(AnimatedScale)).scale;

void main() {
  Widget app(Widget child) => MaterialApp(home: Scaffold(body: child));

  testWidgets('a quick tap sinks for long enough to be seen', (tester) async {
    await tester.pumpWidget(
      app(
        Center(
          child: NfButton(label: 'Play', onPressed: () {}),
        ),
      ),
    );

    await tester.tap(find.byType(NfButton));
    await tester.pump();
    expect(_targetScale(tester), lessThan(1));

    await tester.pump(Press.sink);
    expect(_targetScale(tester), 1);
  });

  testWidgets('a scroll that takes over the gesture lifts the control', (
    tester,
  ) async {
    await tester.pumpWidget(
      app(
        ListView(
          children: [
            NfButton(label: 'Play', onPressed: () {}),
            const SizedBox(height: 2000),
          ],
        ),
      ),
    );

    final gesture = await tester.startGesture(
      tester.getCenter(find.byType(NfButton)),
    );
    await tester.pump(const Duration(milliseconds: 150));
    expect(_targetScale(tester), lessThan(1));

    await gesture.moveBy(const Offset(0, -80));
    await tester.pump(Press.sink);
    expect(_targetScale(tester), 1);

    await gesture.up();
    await tester.pumpAndSettle();
  });
}
