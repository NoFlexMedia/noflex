// SPDX-License-Identifier: AGPL-3.0-only
//
// An account deleted on the server: what of it this device forgets, and whose
// it keeps. The last administrator's takes everyone's, and waits out the
// server's reset.

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
  late DeviceIdentity identity;
  late SessionService session;
  late bool resetting;

  setUp(() async {
    resetting = true;
    FlutterSecureStorage.setMockInitialValues({});
    SharedPreferences.setMockInitialValues({'noflex.origin': 'http://server'});
    // A server that's erasing itself until told otherwise, back at first-run
    // setup after.
    final server = MockClient(
      (_) async => http.Response(
        jsonEncode({
          'data': {
            'serverInfo': {
              'name': 'noflex',
              'version': '0',
              'isConfigured': false,
              'resetting': resetting,
            },
          },
        }),
        200,
      ),
    );
    tokens = TokenStore(client: server);
    await tokens.load();
    identity = DeviceIdentity(tokens);
    await identity.load();
    session = SessionService(
      tokens: tokens,
      graphql: GraphqlService(tokens, httpClient: server),
      identity: identity,
      decoders: DecoderProbeService(),
      watchState: WatchStateService(),
    );
    getIt
      ..registerSingleton(NavService())
      ..registerLazySingleton(HomeService.new)
      ..registerLazySingleton(IndexingService.new);
  });
  tearDown(getIt.reset);

  Future<String?> storedId(String userId) async =>
      (await SharedPreferences.getInstance()).getString(deviceIdKey(userId));

  test('only the deleted account leaves the device', () async {
    await tokens.signIn(account('Kid'));
    await tokens.signIn(account('Vera'));
    final kidsId = identity.idFor('Kid');
    identity.idFor('Vera');

    await session.accountDeleted();

    expect(tokens.accounts.value.map((it) => it.userId), ['Kid']);
    expect(await storedId('Vera'), isNull);
    expect(await storedId('Kid'), kidsId);
    expect(session.phase.value, AppPhase.profiles);
    expect(
      session.authError.value,
      'Vera’s account was deleted, with everything of theirs on this device.',
    );
  });

  test(
    'an erased server takes everyone, and setup follows its reset',
    () async {
      await tokens.signIn(account('Kid'));
      await tokens.signIn(account('Vera'));
      identity.idFor('Kid');

      await session.serverErased();

      expect(tokens.accounts.value, isEmpty);
      expect(tokens.origin.value, 'http://server');
      expect(await storedId('Kid'), isNull);
      expect(session.phase.value, AppPhase.resetting);

      resetting = false;
      await Future<void>.delayed(const Duration(seconds: 3));
      expect(session.phase.value, AppPhase.setup);
    },
  );
}
