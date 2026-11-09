// SPDX-License-Identifier: AGPL-3.0-only
//
// A session the server ended: who is told, and where the app goes. Nobody is
// taken off the device for it.

import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:noflex/di.dart';
import 'package:noflex/features/home/home_service.dart';
import 'package:noflex/features/player/decoder_probe.dart';
import 'package:noflex/services/device_identity.dart';
import 'package:noflex/services/graphql.dart';
import 'package:noflex/services/indexing_service.dart';
import 'package:noflex/services/nav_service.dart';
import 'package:noflex/services/session.dart';
import 'package:noflex/services/tokens.dart';
import 'package:noflex/services/watch_state.dart';
import 'package:shared_preferences/shared_preferences.dart';

StoredAccount account(String name) =>
    StoredAccount(userId: name, name: name, access: 'a', refresh: 'r-$name');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late TokenStore tokens;
  late SessionService session;
  late String code;

  setUp(() async {
    code = 'REFRESH_TOKEN_INVALID';
    FlutterSecureStorage.setMockInitialValues({});
    SharedPreferences.setMockInitialValues({'noflex.origin': 'http://server'});
    // A server that refuses whatever it's asked.
    final server = MockClient(
      (_) async => http.Response(
        jsonEncode({
          'errors': [
            {
              'message': 'refused',
              'extensions': {'code': code},
            },
          ],
        }),
        200,
      ),
    );
    tokens = TokenStore(client: server);
    await tokens.load();
    session = SessionService(
      tokens: tokens,
      graphql: GraphqlService(tokens, httpClient: server),
      identity: DeviceIdentity(tokens),
      decoders: DecoderProbeService(),
      watchState: WatchStateService(),
    );
    getIt
      ..registerSingleton(NavService())
      ..registerLazySingleton(HomeService.new)
      ..registerLazySingleton(IndexingService.new);
  });
  tearDown(getIt.reset);

  test(
    'with others here, it’s back to the tiles, saying whose ended',
    () async {
      await tokens.signIn(account('Kid'));
      await tokens.signIn(account('Vera'));

      await expectLater(tokens.refresh(), throwsA(isA<SessionExpired>()));
      await pumpEventQueue();

      expect(session.phase.value, AppPhase.profiles);
      expect(
        session.authError.value,
        'Vera’s session expired. Sign in again to continue.',
      );
      expect(tokens.account('Vera')?.needsSignIn, isTrue);
    },
  );

  test(
    'one person goes straight to signing in again, and can back out',
    () async {
      code = 'REFRESH_TOKEN_REUSED';
      await tokens.signIn(account('Vera'));

      await expectLater(tokens.refresh(), throwsA(isA<SessionExpired>()));
      await pumpEventQueue();

      expect(session.phase.value, AppPhase.auth);
      expect(session.signingInFor.value, 'Vera');
      expect(
        session.authError.value,
        'Vera was signed out because their session was used from somewhere '
        'else. Sign in again, and change the password if that wasn’t them.',
      );

      session.cancelAddProfile();
      expect(session.phase.value, AppPhase.profiles);
      expect(tokens.accounts.value.single.needsSignIn, isTrue);

      // Removing them is the deliberate act that takes someone off a device.
      await session.removeProfile('Vera');
      expect(tokens.accounts.value, isEmpty);
      expect(session.phase.value, AppPhase.auth);
      expect(session.signingInFor.value, isNull);
    },
  );
}
