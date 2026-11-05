// SPDX-License-Identifier: AGPL-3.0-only
//
// The server this app talks to and its session, with single-flight refresh.
// Free of GraphQL types so every transport (HTTP, WebSocket, the headless
// download sync) shares one copy of the session.

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

/// `reused` means the server saw a replayed refresh token and revoked the session.
class SessionExpired implements Exception {
  const SessionExpired({this.reused = false});
  final bool reused;
  @override
  String toString() => reused
      ? 'You were signed out because this session was used from somewhere else.'
      : 'Your session has expired.';
}

const _originKey = 'noflex.origin';
const _accessKey = 'noflex.accessToken';
const _refreshKey = 'noflex.refreshToken';
const _refreshMutation =
    'mutation Refresh(\$token: String!) { refreshSession(refreshToken: \$token) { token refreshToken } }';

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
  String? _access;
  String? _refresh;
  Future<void>? _refreshing;

  /// Told once when the server ends the session.
  void Function(SessionExpired error)? onExpired;

  bool get hasSession => _refresh != null;
  String? get accessToken => _access;
  String get graphqlUrl => '${origin.value}/graphql';
  String get websocketUrl =>
      '${origin.value!.replaceFirst(RegExp('^http'), 'ws')}/graphql/ws';

  Map<String, String> get authHeaders =>
      _access == null ? const {} : {'Authorization': 'Bearer $_access'};

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
      _access = await _storage.read(key: _accessKey);
      _refresh = await _storage.read(key: _refreshKey);
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

  Future<void> save({required String access, required String refresh}) async {
    _access = access;
    _refresh = refresh;
    try {
      await _storage.write(key: _accessKey, value: access);
      await _storage.write(key: _refreshKey, value: refresh);
    } catch (error) {
      debugPrint('secure storage unavailable: $error');
    }
  }

  Future<void> clear() async {
    _access = null;
    _refresh = null;
    try {
      await _storage.delete(key: _accessKey);
      await _storage.delete(key: _refreshKey);
    } catch (error) {
      debugPrint('secure storage unavailable: $error');
    }
  }

  /// Rotate the token pair. Single-flight: a second refresh with the same
  /// token looks like replay to the server. Pass the access token a rejected
  /// request carried to skip refreshing when another caller already replaced
  /// it. A refusal clears the session and throws [SessionExpired]; an
  /// unreachable server throws [ServerUnreachable] and keeps it.
  Future<void> refresh([String? staleAccess]) {
    if (staleAccess != null && _access != null && staleAccess != _access) {
      return Future.value();
    }
    return _refreshing ??= _doRefresh().whenComplete(() => _refreshing = null);
  }

  Future<void> _doRefresh() async {
    // Another isolate holds its own copy of the session and may have rotated it.
    await reloadTokens();
    final token = _refresh;
    if (token == null) return _expire(const SessionExpired());
    try {
      final tokens = await _redeem(token);
      await save(access: tokens.$1, refresh: tokens.$2);
    } on ApiError catch (error) {
      // A sign-in that finished mid-refresh owns the session now.
      if (_refresh != token) return;
      return _expire(
        SessionExpired(reused: error.codes.contains('REFRESH_TOKEN_REUSED')),
      );
    }
  }

  Future<void> _expire(SessionExpired error) async {
    await clear();
    onExpired?.call(error);
    throw error;
  }

  Future<(String, String)> _redeem(String token) async {
    final http.Response response;
    try {
      response = await _client
          .post(
            Uri.parse(graphqlUrl),
            headers: const {'Content-Type': 'application/json'},
            body: jsonEncode({
              'query': _refreshMutation,
              'variables': {'token': token},
            }),
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
      throw ApiError(errors.map((e) => (e as Map)['message']).join('; '), [
        for (final e in errors)
          if ((e as Map)['extensions'] case {'code': final String code}) code,
      ]);
    }
    final result = (payload['data'] as Map?)?['refreshSession'] as Map?;
    if (result == null) {
      throw const ServerUnreachable('The server returned an empty response.');
    }
    return (result['token'] as String, result['refreshToken'] as String);
  }
}
