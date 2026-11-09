// SPDX-License-Identifier: AGPL-3.0-only
//
// The server this app talks to and the sessions it holds there: one for each
// person signed in on this device, of which one at most is active. Free of
// GraphQL types so every transport (HTTP, WebSocket, the headless download
// sync) shares one copy.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:signals_flutter/signals_flutter.dart';

/// The server answered and refused, as opposed to being unreachable.
class ApiError implements Exception {
  const ApiError(this.message, [this.codes = const []]);
  final String message;
  final List<String> codes;
  @override
  String toString() => message;
}

class ServerUnreachable implements Exception {
  const ServerUnreachable([
    this.message = 'NoFlex could not reach the server. Check your connection and try again.',
  ]);
  final String message;
  @override
  String toString() => message;
}

/// The session ended, or stopped being usable: `reused` means the server saw a
/// replayed refresh token and revoked it, `locked` that it's waiting for its
/// profile's PIN and is otherwise intact.
class SessionExpired implements Exception {
  const SessionExpired({this.reused = false, this.locked = false});
  final bool reused;
  final bool locked;
  @override
  String toString() => locked
      ? 'This profile is locked.'
      : reused
      ? 'You were signed out because this session was used from somewhere else.'
      : 'Your session has expired.';
}

/// The PIN didn't open the profile, and the session has [attemptsLeft] more
/// before the server ends it.
class WrongPin implements Exception {
  const WrongPin(this.attemptsLeft);
  final int attemptsLeft;
  @override
  String toString() => attemptsLeft == 1
      ? 'That PIN isn’t right. 1 try left.'
      : 'That PIN isn’t right. $attemptsLeft tries left.';
}

/// Someone signed in on this device: what a tile shows of them, and the
/// session that's theirs.
class StoredAccount {
  const StoredAccount({
    required this.userId,
    required this.name,
    required this.access,
    required this.refresh,
    this.hasPin = false,
    this.avatarUrl,
    this.needsSignIn = false,
  });

  factory StoredAccount.fromJson(Map<String, dynamic> json) => StoredAccount(
    userId: json['userId'] as String,
    name: json['name'] as String,
    access: json['access'] as String,
    refresh: json['refresh'] as String,
    hasPin: json['hasPin'] as bool? ?? false,
    avatarUrl: json['avatarUrl'] as String?,
    needsSignIn: json['needsSignIn'] as bool? ?? false,
  );

  final String userId;
  final String name;
  final String access;
  final String refresh;

  /// Choosing them goes through [TokenStore.unlock].
  final bool hasPin;

  /// As the server last signed it, relative to the server.
  final String? avatarUrl;

  /// The server no longer knows the session. The tile stays, so they can
  /// sign in again and find what was theirs.
  final bool needsSignIn;

  StoredAccount copyWith({
    String? name,
    String? access,
    String? refresh,
    bool? hasPin,
    String? avatarUrl,
    bool clearAvatar = false,
    bool? needsSignIn,
  }) => StoredAccount(
    userId: userId,
    name: name ?? this.name,
    access: access ?? this.access,
    refresh: refresh ?? this.refresh,
    hasPin: hasPin ?? this.hasPin,
    avatarUrl: clearAvatar ? null : avatarUrl ?? this.avatarUrl,
    needsSignIn: needsSignIn ?? this.needsSignIn,
  );

  Map<String, dynamic> toJson() => {
    'userId': userId,
    'name': name,
    'access': access,
    'refresh': refresh,
    'hasPin': hasPin,
    'avatarUrl': avatarUrl,
    'needsSignIn': needsSignIn,
  };
}

const _originKey = 'noflex.origin';
const _sessionKey = 'noflex.session';
const _refreshMutation =
    'mutation Refresh(\$token: String!) { refreshSession(refreshToken: \$token) { token refreshToken } }';
const _lockMutation =
    'mutation Lock(\$token: String!) { lockSession(refreshToken: \$token) }';
const _unlockMutation =
    'mutation Unlock(\$token: String!, \$pin: String!) { unlockSession(refreshToken: \$token, pin: \$pin) { token refreshToken } }';

class TokenStore {
  TokenStore({FlutterSecureStorage? storage, http.Client? client})
    : _storage = storage ?? _defaultStorage,
      _client = client ?? http.Client();

  // first_unlock: a background wake may refresh while the phone is locked.
  static const _defaultStorage = FlutterSecureStorage(
    iOptions: IOSOptions(accessibility: KeychainAccessibility.first_unlock),
    mOptions: MacOsOptions(accessibility: KeychainAccessibility.first_unlock),
  );

  final FlutterSecureStorage _storage;
  final http.Client _client;

  final origin = signal<String?>(null);

  /// Everyone signed in on this device, in the order they first did.
  final accounts = signal<List<StoredAccount>>(const []);

  /// Whose session requests carry. Null on the way in, with nobody chosen.
  final activeId = signal<String?>(null);

  Future<void>? _refreshing;

  /// Told once when the server ends the active session. The account is gone
  /// from [accounts] by then.
  void Function(SessionExpired error)? onExpired;

  /// Told when the active session turns out to be waiting for its PIN.
  void Function()? onLocked;

  StoredAccount? account(String? userId) {
    if (userId == null) return null;
    for (final account in accounts.value) {
      if (account.userId == userId) return account;
    }
    return null;
  }

  StoredAccount? get active => account(activeId.value);
  bool get hasSession => active != null;
  String? get accessToken => active?.access;
  String get graphqlUrl => '${origin.value}/graphql';
  String get websocketUrl =>
      '${origin.value!.replaceFirst(RegExp('^http'), 'ws')}/graphql/ws';

  Map<String, String> get authHeaders {
    final access = accessToken;
    return access == null ? const {} : {'Authorization': 'Bearer $access'};
  }

  /// A URL from a response as this app must request it: responses use paths
  /// relative to the server that sent them.
  String absolute(String url) =>
      url.startsWith('/') ? '${origin.value}$url' : url;

  Future<void> load() async {
    final prefs = await SharedPreferences.getInstance();
    origin.value = prefs.getString(_originKey);
    await reloadTokens();
  }

  /// The headless sync may have rotated the tokens since they were read.
  Future<void> reloadTokens() async {
    try {
      final stored = await _storage.read(key: _sessionKey);
      final json = stored == null
          ? const <String, dynamic>{}
          : jsonDecode(stored) as Map<String, dynamic>;
      final loaded = [
        for (final account in json['accounts'] as List? ?? const [])
          StoredAccount.fromJson((account as Map).cast<String, dynamic>()),
      ];
      batch(() {
        accounts.value = loaded;
        activeId.value = json['active'] as String?;
      });
    } catch (error) {
      // A Linux session without a keyring: stay signed in for this run only.
      debugPrint('secure storage unavailable: $error');
    }
  }

  Future<void> setOrigin(String? value) async {
    final prefs = await SharedPreferences.getInstance();
    if (value == null) {
      await prefs.remove(_originKey);
    } else {
      await prefs.setString(_originKey, value);
    }
    origin.value = value;
  }

  // Accounts and who's active go in one value, so the other isolate never reads
  // one without the other.
  Future<void> _set(List<StoredAccount> next, String? active) async {
    batch(() {
      accounts.value = next;
      activeId.value = active;
    });
    try {
      await _storage.write(
        key: _sessionKey,
        value: jsonEncode({
          'active': active,
          'accounts': [for (final account in next) account.toJson()],
        }),
      );
    } catch (error) {
      debugPrint('secure storage unavailable: $error');
    }
  }

  Future<void> _replace(StoredAccount next) => _set([
    for (final account in accounts.value)
      account.userId == next.userId ? next : account,
  ], activeId.value);

  /// Remember a session just issued and make it the active one. Someone
  /// already here keeps their place: the new session replaces the old.
  Future<void> signIn(StoredAccount account) {
    final known = this.account(account.userId) != null;
    return _set([
      for (final other in accounts.value)
        other.userId == account.userId ? account : other,
      if (!known) account,
    ], account.userId);
  }

  /// The active session's tokens, after the server rotated them.
  Future<void> save({required String access, required String refresh}) async {
    final current = active;
    if (current == null) return;
    await _replace(current.copyWith(access: access, refresh: refresh));
  }

  /// What the server says of someone now. Null leaves a field as it is.
  Future<void> describe(
    String userId, {
    String? name,
    bool? hasPin,
    String? avatarUrl,
    bool clearAvatar = false,
    bool? needsSignIn,
  }) async {
    final current = account(userId);
    if (current == null) return;
    await _replace(
      current.copyWith(
        name: name,
        hasPin: hasPin,
        avatarUrl: avatarUrl,
        clearAvatar: clearAvatar,
        needsSignIn: needsSignIn,
      ),
    );
  }

  Future<void> activate(String userId) async {
    if (account(userId) != null) await _set(accounts.value, userId);
  }

  /// Nobody's session is in use, as on "Who's watching?".
  Future<void> deactivate() => _set(accounts.value, null);

  Future<void> remove(String userId) => _set([
    for (final account in accounts.value)
      if (account.userId != userId) account,
  ], activeId.value == userId ? null : activeId.value);

  /// Forget the active account, leaving the rest.
  Future<void> removeActive() async {
    final userId = activeId.value;
    if (userId != null) await remove(userId);
  }

  /// Forget everyone: the sessions belong to a server being left.
  Future<void> clear() => _set(const [], null);

  /// Rotate the active token pair. Single-flight: a second refresh with the same
  /// token looks like replay to the server. Pass the access token a rejected
  /// request carried to skip refreshing when another caller already replaced
  /// it. A refusal forgets the account and throws [SessionExpired], unless the
  /// session is only locked, which keeps it; an unreachable server throws
  /// [ServerUnreachable] and keeps it.
  Future<void> refresh([String? staleAccess]) {
    final access = accessToken;
    if (staleAccess != null && access != null && staleAccess != access) {
      return Future.value();
    }
    return _refreshing ??= _doRefresh().whenComplete(() => _refreshing = null);
  }

  Future<void> _doRefresh() async {
    // Another isolate holds its own copy of the session and may have rotated it.
    await reloadTokens();
    final userId = activeId.value;
    final token = active?.refresh;
    if (token == null) return _expire(userId, const SessionExpired());
    try {
      final tokens = await _redeem(_refreshMutation, 'refreshSession', {
        'token': token,
      });
      // Whoever was active when this was asked, which may no longer be so.
      final current = account(userId);
      if (current?.refresh == token) {
        await _replace(
          current!.copyWith(access: tokens.$1, refresh: tokens.$2),
        );
      }
    } on ApiError catch (error) {
      // A sign-in that finished mid-refresh owns the session now.
      if (account(userId)?.refresh != token) return;
      if (error.codes.contains('SESSION_LOCKED')) {
        if (activeId.value == userId) onLocked?.call();
        throw const SessionExpired(locked: true);
      }
      return _expire(
        userId,
        SessionExpired(reused: error.codes.contains('REFRESH_TOKEN_REUSED')),
      );
    }
  }

  Future<void> _expire(String? userId, SessionExpired error) async {
    final wasActive = activeId.value == userId;
    if (userId != null) await remove(userId);
    if (wasActive) onExpired?.call(error);
    throw error;
  }

  /// Leave someone's session waiting for their PIN. Best effort: a device
  /// asks for the PIN on the way back in whether or not this got through.
  Future<void> lock(String userId) async {
    final token = account(userId)?.refresh;
    if (token == null) return;
    // A refresh under way would rotate the token out from under the lock.
    try {
      await _refreshing;
    } catch (_) {}
    try {
      await _post(_lockMutation, {'token': account(userId)?.refresh ?? token});
    } catch (_) {
      // Unreachable, or already gone.
    }
  }

  /// Open someone's session with their PIN, taking the fresh tokens. Throws
  /// [WrongPin], an [ApiError] carrying the server's code when the session
  /// can't be opened at all, or [ServerUnreachable].
  Future<void> unlock(String userId, String pin) async {
    final current = account(userId);
    if (current == null) return;
    final tokens = await _redeem(_unlockMutation, 'unlockSession', {
      'token': current.refresh,
      'pin': pin,
    });
    final latest = account(userId);
    if (latest?.refresh != current.refresh) return;
    await _replace(latest!.copyWith(access: tokens.$1, refresh: tokens.$2));
  }

  Future<(String, String)> _redeem(
    String mutation,
    String field,
    Map<String, String> variables,
  ) async {
    final result = (await _post(mutation, variables))[field] as Map?;
    if (result == null) {
      throw const ServerUnreachable('The server returned an empty response.');
    }
    return (result['token'] as String, result['refreshToken'] as String);
  }

  Future<Map<String, dynamic>> _post(
    String query,
    Map<String, String> variables,
  ) async {
    final http.Response response;
    try {
      response = await _client
          .post(
            Uri.parse(graphqlUrl),
            headers: const {'Content-Type': 'application/json'},
            body: jsonEncode({'query': query, 'variables': variables}),
          )
          .timeout(const Duration(seconds: 20));
    } catch (_) {
      throw const ServerUnreachable();
    }
    if (response.statusCode >= 400 && response.statusCode < 500) {
      throw ApiError('The server returned ${response.statusCode}.');
    }
    if (response.statusCode >= 500) throw const ServerUnreachable();
    final payload = jsonDecode(response.body) as Map<String, dynamic>;
    final errors = payload['errors'] as List?;
    if (errors != null && errors.isNotEmpty) {
      for (final error in errors) {
        if ((error as Map)['extensions'] case {
          'code': 'PIN_INCORRECT',
          'attemptsLeft': final int left,
        }) {
          throw WrongPin(left);
        }
      }
      throw ApiError(errors.map((e) => (e as Map)['message']).join('; '), [
        for (final e in errors)
          if ((e as Map)['extensions'] case {'code': final String code}) code,
      ]);
    }
    return (payload['data'] as Map?)?.cast<String, dynamic>() ?? const {};
  }
}
