// SPDX-License-Identifier: AGPL-3.0-only
//
// Several people signed in on one device: whose session is whose, and what the
// server refusing one of them does to the rest.

import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:noflex/services/tokens.dart';
import 'package:shared_preferences/shared_preferences.dart';

StoredAccount account(String id, {String refresh = 'r1', bool pin = false}) =>
    StoredAccount(
      userId: id,
      name: id,
      access: 'access-$id',
      refresh: '$id-$refresh',
      hasPin: pin,
    );

/// A server answering every request with [answer]'s GraphQL payload.
Future<TokenStore> store(
  Map<String, Object?> Function(Map<String, dynamic> variables) answer,
) async {
  final tokens = TokenStore(
    client: MockClient((request) async {
      final body = jsonDecode(request.body) as Map<String, dynamic>;
      return http.Response(
        jsonEncode(answer(body['variables'] as Map<String, dynamic>)),
        200,
      );
    }),
  );
  await tokens.load();
  return tokens;
}

Map<String, Object?> refused(String code, [Map<String, Object?>? more]) => {
  'errors': [
    {
      'message': 'refused',
      'extensions': {'code': code, ...?more},
    },
  ],
};

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  setUp(() {
    FlutterSecureStorage.setMockInitialValues({});
    SharedPreferences.setMockInitialValues({'noflex.origin': 'http://server'});
  });

  test(
    'signing in again replaces a session where it was, and it all persists',
    () async {
      final tokens = await store((_) => {});
      await tokens.signIn(account('vera'));
      await tokens.signIn(account('kid'));
      await tokens.signIn(account('vera', refresh: 'r2'));

      expect(tokens.accounts.value.map((a) => a.userId), ['vera', 'kid']);
      expect(tokens.active?.refresh, 'vera-r2');

      // As the headless sync reads it, from its own isolate.
      final other = await store((_) => {});
      expect(other.accounts.value.map((a) => a.refresh), ['vera-r2', 'kid-r1']);
      expect(other.activeId.value, 'vera');

      await tokens.removeActive();
      expect(tokens.hasSession, isFalse);
      expect(tokens.accounts.value.single.userId, 'kid');
    },
  );

  test('a locked session is kept, and says so', () async {
    final tokens = await store((_) => refused('SESSION_LOCKED'));
    await tokens.signIn(account('vera', pin: true));
    var locked = 0;
    tokens.onLocked = () => locked++;
    tokens.onExpired = (_) => fail('a lock is not an expiry');

    await expectLater(
      tokens.refresh(),
      throwsA(isA<SessionExpired>().having((e) => e.locked, 'locked', isTrue)),
    );
    expect(locked, 1);
    expect(tokens.active?.refresh, 'vera-r1');
  });

  test('a session the server ended takes only its own account', () async {
    final tokens = await store((_) => refused('REFRESH_TOKEN_INVALID'));
    await tokens.signIn(account('kid'));
    await tokens.signIn(account('vera'));
    final expired = <SessionExpired>[];
    tokens.onExpired = expired.add;

    await expectLater(tokens.refresh(), throwsA(isA<SessionExpired>()));
    expect(expired, hasLength(1));
    expect(tokens.hasSession, isFalse);
    expect(tokens.accounts.value.single.userId, 'kid');
  });

  test('a PIN opens a session without making it the active one', () async {
    final tokens = await store(
      (variables) => variables['pin'] == '1234'
          ? {
              'data': {
                'unlockSession': {'token': 'fresh', 'refreshToken': 'vera-r2'},
              },
            }
          : refused('PIN_INCORRECT', {'attemptsLeft': 4}),
    );
    await tokens.signIn(account('vera', pin: true));
    await tokens.signIn(account('kid'));

    await expectLater(
      tokens.unlock('vera', '0000'),
      throwsA(isA<WrongPin>().having((e) => e.attemptsLeft, 'left', 4)),
    );
    expect(tokens.account('vera')?.refresh, 'vera-r1');

    await tokens.unlock('vera', '1234');
    expect(tokens.account('vera')?.access, 'fresh');
    expect(tokens.account('vera')?.refresh, 'vera-r2');
    expect(tokens.activeId.value, 'kid');
  });
}
