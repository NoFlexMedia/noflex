// SPDX-License-Identifier: AGPL-3.0-only
//
// Linking another server: sign in there once, have it mint a revocable key, and
// keep only the key. The web splits this over a page reload for its security
// policy, which a native client doesn't have. This talks to a server that isn't
// linked yet, with a token worth keeping for one sequence, so it goes through a
// throwaway client instead of the service's cached per-server ones.

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:gql_http_link/gql_http_link.dart';

import '../../di.dart';
import '../../graphql/queries/__generated__/add_linked_server.req.gql.dart';
import '../../graphql/queries/__generated__/create_api_key.req.gql.dart';
import '../../graphql/queries/__generated__/link_login.req.gql.dart';
import '../../graphql/queries/__generated__/logout.req.gql.dart';
import '../../services/device_identity.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../../services/tokens.dart';
import '../../widgets/nf_overlays.dart';

/// What every server calls itself until someone names it, which would tell
/// linked servers apart from nothing.
const _defaultServerName = 'noflex';

Future<TData> _askServer<TData, TVars>(
  String origin,
  OperationRequest<TData, TVars> request, {
  String? token,
}) async {
  final client = Client(
    link: HttpLink(
      '$origin/graphql',
      defaultHeaders: {if (token != null) 'Authorization': 'Bearer $token'},
    ),
    cache: Cache(),
  );
  try {
    final response = await client
        .request(request)
        .firstWhere((r) => !r.loading || r.hasErrors)
        .timeout(
          const Duration(seconds: 20),
          onTimeout: () => throw const ServerUnreachable(),
        );
    throwIfFailed(response);
    final data = response.data;
    if (data == null) {
      throw const ServerUnreachable('The server returned an empty response.');
    }
    return data;
  } finally {
    await client.dispose();
  }
}

/// Link [origin] with the account given. Answers with the server's name, or
/// null if the viewer backed out. Throws what any request does, or on a refusal.
Future<String?> linkServer({
  required String origin,
  required String username,
  required String password,
}) async {
  final graphql = getIt<GraphqlService>();
  final home = getIt<SessionService>().serverName.value ?? 'this server';
  final info = await graphql.probe(origin);
  final signedIn = await _askServer(
    origin,
    GLinkLoginReq(
      (b) => b.vars
        ..username = username
        ..password = password
        ..device = getIt<DeviceIdentity>().name,
    ),
  );
  final token = signedIn.login.token;
  try {
    if (signedIn.login.user.isAdmin) {
      final proceed = await confirmAction(
        title: 'Link an administrator account?',
        message:
            'A key for this account will be stored on $home, so anyone who gets into it could administer ${info.name}. An ordinary account is safer.',
        confirmLabel: 'Link anyway',
        danger: true,
      );
      if (!proceed) return null;
    }
    final created = await _askServer(
      origin,
      GCreateApiKeyReq((b) => b.vars.name = 'NoFlex link from $home'),
      token: token,
    );
    final name = info.name != _defaultServerName
        ? info.name
        : Uri.parse(origin).host;
    await graphql.run(
      GAddLinkedServerReq(
        (b) => b.vars
          ..origin = origin
          ..name = name
          ..key = created.createApiKey.key,
      ),
    );
    return name;
  } finally {
    // The key is what stays; this sign-in has done its job.
    unawaited(
      _askServer(
        origin,
        GLogoutReq(),
        token: token,
      ).then<void>((_) {}, onError: (Object _) {}),
    );
  }
}
