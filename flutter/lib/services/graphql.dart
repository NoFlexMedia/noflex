// SPDX-License-Identifier: AGPL-3.0-only
//
// Ferry clients: one for the home server, whose session refreshes, and one per
// linked server, whose API key doesn't. Failures surface as the types in
// tokens.dart so callers can tell "unreachable" from "refused".

import 'dart:async';
import 'dart:convert';

import 'package:ferry/ferry.dart';
import 'package:gql_exec/gql_exec.dart';
import 'package:gql_http_link/gql_http_link.dart';
import 'package:gql_websocket_link/gql_websocket_link.dart';
import 'package:http/http.dart' as http;
import 'package:web_socket_channel/web_socket_channel.dart';

import '../models/work.dart';
import 'tokens.dart';

final _authError = RegExp(
  'authentication required|invalid token|token expired|unauthenticated',
  caseSensitive: false,
);

class ServerInfoProbe {
  const ServerInfoProbe({
    required this.name,
    required this.version,
    required this.isConfigured,
  });
  final String name;
  final String version;
  final bool isConfigured;
}

class GraphqlService {
  GraphqlService(this.tokens, {http.Client? httpClient})
    : _httpClient = httpClient ?? http.Client();

  final TokenStore tokens;
  final http.Client _httpClient;
  _Transport? _home;
  final _linked = <String, _Transport>{};

  /// The home server's client.
  Client get client {
    final origin = tokens.origin.value!;
    if (_home?.origin != origin) {
      unawaited(_home?.dispose());
      _home = _Transport.home(tokens, origin, _httpClient);
    }
    return _home!.client;
  }

  Client clientFor(ServerRef? server) {
    if (server == null) return client;
    return (_linked[server.id] ??= _Transport.linked(
      server,
      _httpClient,
    )).client;
  }

  /// One operation to its final answer. Throws [ApiError], [ServerUnreachable]
  /// or [SessionExpired]. Requests should carry `FetchPolicy.NetworkOnly`
  /// where a cached answer would be wrong; mutations do by default.
  Future<TData> run<TData, TVars>(
    OperationRequest<TData, TVars> request, {
    ServerRef? server,
    Duration timeout = const Duration(seconds: 30),
  }) async {
    final response = await clientFor(server)
        .request(request)
        .firstWhere((r) => !r.loading || r.hasErrors)
        .timeout(timeout, onTimeout: () => throw const ServerUnreachable());
    throwIfFailed(response);
    final data = response.data;
    if (data == null) {
      throw const ServerUnreachable('The server returned an empty response.');
    }
    return data;
  }

  /// Forget everything cached for the signed-in user.
  void reset() {
    unawaited(_home?.dispose());
    _home = null;
    for (final transport in _linked.values) {
      unawaited(transport.dispose());
    }
    _linked.clear();
  }

  /// Ask a server about itself before committing to it.
  Future<ServerInfoProbe> probe(String origin) async {
    final http.Response response;
    try {
      response = await _httpClient
          .post(
            Uri.parse('$origin/graphql'),
            headers: const {'Content-Type': 'application/json'},
            body: jsonEncode({
              'query': '{ serverInfo { name version isConfigured } }',
            }),
          )
          .timeout(const Duration(seconds: 10));
    } catch (_) {
      throw const ServerUnreachable('Nothing answered at that address.');
    }
    if (response.statusCode != 200) {
      throw ApiError(
        'That address answered with ${response.statusCode}, not NoFlex.',
      );
    }
    try {
      final info =
          ((jsonDecode(response.body) as Map)['data'] as Map)['serverInfo']
              as Map;
      return ServerInfoProbe(
        name: info['name'] as String,
        version: info['version'] as String,
        isConfigured: info['isConfigured'] as bool,
      );
    } catch (_) {
      throw const ApiError(
        'That address answered, but it isn’t a NoFlex server.',
      );
    }
  }
}

/// A response's failure as an exception, if it failed.
void throwIfFailed(OperationResponse<dynamic, dynamic> response) {
  final failure = failureOf(response);
  if (failure != null) throw failure;
}

Exception? failureOf(OperationResponse<dynamic, dynamic> response) {
  final link = response.linkException;
  if (link != null) {
    final cause = link.originalException;
    if (cause is SessionExpired) return cause;
    if (cause is ApiError) return cause;
    if (link is HttpLinkServerException) {
      final status = link.response.statusCode;
      if (status >= 400 && status < 500) {
        return ApiError('The server returned $status.');
      }
    }
    return const ServerUnreachable();
  }
  final errors = response.graphqlErrors;
  if (errors != null && errors.isNotEmpty) {
    return ApiError(errors.map((e) => e.message).join('; '), [
      for (final error in errors)
        if (error.extensions?['code'] case final String code) code,
    ]);
  }
  return null;
}

class _Transport {
  _Transport._(
    this.origin,
    Link auth,
    http.Client httpClient,
    String Function() bearer,
  ) {
    final socket = Uri.parse(
      '${origin.replaceFirst(RegExp('^http'), 'ws')}/graphql/ws',
    );
    _ws = WebSocketLink(
      null,
      // The server refuses an upgrade that doesn't name a subprotocol, and this
      // link speaks the older of the two it offers. Awaiting `ready` hands a
      // refused socket to the link's reconnecting rather than leaving it
      // unhandled.
      channelGenerator: () async {
        final channel = WebSocketChannel.connect(
          socket,
          protocols: const ['graphql-ws'],
        );
        await channel.ready;
        return channel;
      },
      autoReconnect: true,
      reconnectInterval: const Duration(seconds: 3),
      initialPayload: () async => {'Authorization': 'Bearer ${bearer()}'},
    );
    final link = Link.split(
      (request) =>
          request.operation.getOperationType() == OperationType.subscription,
      _ws,
      auth.concat(HttpLink('$origin/graphql', httpClient: httpClient)),
    );
    client = Client(
      link: link,
      cache: Cache(),
      defaultFetchPolicies: const {
        OperationType.query: FetchPolicy.CacheAndNetwork,
      },
    );
  }

  factory _Transport.home(
    TokenStore tokens,
    String origin,
    http.Client httpClient,
  ) => _Transport._(
    origin,
    _HomeAuthLink(tokens),
    httpClient,
    () => tokens.accessToken ?? '',
  );

  factory _Transport.linked(ServerRef server, http.Client httpClient) =>
      _Transport._(
        server.origin,
        _BearerLink(server.apiKey),
        httpClient,
        () => server.apiKey,
      );

  final String origin;
  late final Client client;
  late final WebSocketLink _ws;

  Future<void> dispose() async {
    await client.dispose();
    await _ws.dispose();
  }
}

Request _withBearer(Request request, String? token) => token == null
    ? request
    : request.updateContextEntry<HttpLinkHeaders>(
        (headers) => HttpLinkHeaders(
          headers: {...?headers?.headers, 'Authorization': 'Bearer $token'},
        ),
      );

class _BearerLink extends Link {
  _BearerLink(this.token);
  final String token;
  @override
  Stream<Response> request(Request request, [NextLink? forward]) =>
      forward!(_withBearer(request, token));
}

/// Bearer header, plus one refresh and retry when the server rejects it.
class _HomeAuthLink extends Link {
  _HomeAuthLink(this.tokens);
  final TokenStore tokens;

  @override
  Stream<Response> request(Request request, [NextLink? forward]) =>
      _send(request, forward!, mayRefresh: true);

  Stream<Response> _send(
    Request request,
    NextLink forward, {
    required bool mayRefresh,
  }) async* {
    final sent = tokens.accessToken;
    try {
      await for (final response in forward(_withBearer(request, sent))) {
        final refused =
            response.errors?.any((e) => _authError.hasMatch(e.message)) ??
            false;
        if (refused && mayRefresh && tokens.hasSession) {
          await tokens.refresh(sent);
          yield* _send(request, forward, mayRefresh: false);
          return;
        }
        yield response;
      }
    } on HttpLinkServerException catch (error) {
      final status = error.response.statusCode;
      if (mayRefresh && tokens.hasSession && (status == 401 || status == 403)) {
        await tokens.refresh(sent);
        yield* _send(request, forward, mayRefresh: false);
        return;
      }
      rethrow;
    }
  }
}
