// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/di.dart';
import 'package:noflex/features/auth/profiles_screen.dart';
import 'package:noflex/features/player/decoder_probe.dart';
import 'package:noflex/services/device_identity.dart';
import 'package:noflex/services/graphql.dart';
import 'package:noflex/services/nav_service.dart';
import 'package:noflex/services/session.dart';
import 'package:noflex/services/toast_service.dart';
import 'package:noflex/services/tokens.dart';
import 'package:noflex/services/watch_state.dart';
import 'package:shared_preferences/shared_preferences.dart';

StoredAccount account(String name, {bool pin = false, bool gone = false}) =>
    StoredAccount(
      userId: name,
      name: name,
      access: 'a',
      refresh: 'r-$name',
      hasPin: pin,
      needsSignIn: gone,
    );

void main() {
  late TokenStore tokens;
  late SessionService session;

  setUp(() async {
    FlutterSecureStorage.setMockInitialValues({});
    SharedPreferences.setMockInitialValues({});
    tokens = TokenStore();
    final graphql = GraphqlService(tokens);
    session = SessionService(
      tokens: tokens,
      graphql: graphql,
      identity: DeviceIdentity(tokens),
      decoders: DecoderProbeService(),
      watchState: WatchStateService(),
    );
    getIt
      ..registerSingleton(tokens)
      ..registerSingleton(NavService())
      ..registerSingleton(ToastService())
      ..registerSingleton(session);
    await tokens.signIn(account('Vera', pin: true));
    await tokens.signIn(account('Kid'));
    await tokens.signIn(account('Gran', gone: true));
    await tokens.deactivate();
  });
  tearDown(getIt.reset);

  testWidgets('a phone fits everyone signed in, and says what each needs', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(360, 640));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(const MaterialApp(home: ProfilesScreen()));

    expect(find.bySemanticsLabel('Vera, locked with a PIN'), findsOneWidget);
    expect(find.bySemanticsLabel('Kid'), findsOneWidget);
    expect(find.text('Sign in again'), findsOneWidget);
    expect(find.text('Add someone'), findsOneWidget);

    // Someone whose session ended signs in again; nobody is let in as them.
    await tester.tap(find.text('Gran'));
    await tester.pump();
    expect(session.phase.value, AppPhase.auth);
    expect(session.signingInFor.value, 'Gran');
    expect(tokens.hasSession, isFalse);
  });

  testWidgets('without the server there’s nobody to add and no PIN to check', (
    tester,
  ) async {
    session.reachable.value = false;
    await tester.pumpWidget(const MaterialApp(home: ProfilesScreen()));

    expect(find.text('Add someone'), findsNothing);
    await tester.tap(find.text('Vera'));
    await tester.pump();
    expect(tokens.hasSession, isFalse);
    expect(getIt<ToastService>().toasts.value.single.kind, ToastKind.error);
    // The toast's timer, which the test would otherwise leave running.
    await tester.pump(const Duration(seconds: 7));
  });
}
