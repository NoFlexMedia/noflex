// SPDX-License-Identifier: AGPL-3.0-only
//
// The app's lifecycle, a port of web/src/lib/session.svelte.ts with two more
// phases a native client needs: choosing a server, there being no same-origin,
// and running offline from what's been downloaded.

import 'dart:async';

import 'package:built_value/json_object.dart';
import 'package:ferry/ferry.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../di.dart';
import '../features/downloads/downloads_service.dart';
import '../features/home/home_service.dart';
import '../features/player/capabilities.dart';
import '../features/player/decoder_probe.dart';
import '../graphql/__generated__/schema.schema.gql.dart';
import '../graphql/queries/__generated__/bootstrap.data.gql.dart';
import '../graphql/queries/__generated__/bootstrap.req.gql.dart';
import '../graphql/queries/__generated__/complete_setup.req.gql.dart';
import '../graphql/queries/__generated__/login.req.gql.dart';
import '../graphql/queries/__generated__/logout.req.gql.dart';
import '../graphql/queries/__generated__/redeem_invite.req.gql.dart';
import '../graphql/queries/__generated__/register_device.req.gql.dart';
import '../graphql/queries/fragments/__generated__/device_fields.data.gql.dart';
import '../graphql/queries/fragments/__generated__/library_fields.data.gql.dart';
import '../graphql/queries/fragments/__generated__/linked_server_fields.data.gql.dart';
import '../graphql/queries/fragments/__generated__/user_fields.data.gql.dart';
import '../models/work.dart';
import '../utils/tracks.dart';
import 'device_identity.dart';
import 'graphql.dart';
import 'nav_service.dart';
import 'tokens.dart';
import 'watch_state.dart';

enum AppPhase { boot, connect, error, setup, auth, app, offline }

/// Preference key: list libraries in the navigation alongside content types.
const showLibrariesKey = 'browse.showLibraries';

class SessionService {
  SessionService({
    required this.tokens,
    required this.graphql,
    required this.identity,
    required this.decoders,
    required this.watchState,
  }) {
    tokens.onExpired = (error) => _signedOut(
      error.reused
          ? '${error.toString()} Sign in again, and change your password if that wasn’t you.'
          : 'Your session expired. Sign in again to continue.',
    );
  }

  final TokenStore tokens;
  final GraphqlService graphql;
  final DeviceIdentity identity;
  final DecoderProbeService decoders;
  final WatchStateService watchState;

  final phase = signal(AppPhase.boot);
  final serverName = signal<String?>(null);
  final serverVersion = signal<String?>(null);
  final serverBackend = signal<String?>(null);
  final viewer = signal<GUserFields?>(null);
  final libraries = signal<List<GLibraryFields>>(const []);
  final categories = signal<Map<GContentCategory, int>>(const {});
  final linkedServers = signal<List<GLinkedServerFields>>(const []);
  final playbackPrefs = signal(const PlaybackPrefs());
  final showLibraries = signal(false);
  final device = signal<GDeviceFields?>(null);
  final error = signal('');
  final authError = signal('');

  /// Linked servers the user has signed in to.
  late final connectedServers = computed(
    () => [
      for (final server in linkedServers.value)
        if (server.apiKey.isNotEmpty)
          ServerRef(
            id: server.id,
            name: server.name,
            origin: server.origin,
            apiKey: server.apiKey,
          ),
    ],
  );

  Future<void> start() async {
    batch(() {
      phase.value = AppPhase.boot;
      error.value = '';
    });
    final origin = tokens.origin.value;
    if (origin == null) {
      phase.value = AppPhase.connect;
      return;
    }
    try {
      final info = await graphql.probe(origin);
      serverName.value = info.name;
      if (!info.isConfigured) {
        phase.value = AppPhase.setup;
        return;
      }
      if (!tokens.hasSession) {
        phase.value = AppPhase.auth;
        return;
      }
      await bootstrap();
    } on ServerUnreachable catch (cause) {
      // Not a sign-out, and what's downloaded still plays.
      if (tokens.hasSession) {
        phase.value = AppPhase.offline;
      } else {
        batch(() {
          error.value = cause.message;
          phase.value = AppPhase.error;
        });
      }
    } on ApiError catch (cause) {
      batch(() {
        error.value = cause.message;
        phase.value = AppPhase.error;
      });
    }
  }

  Future<void> bootstrap() async {
    try {
      final data = await graphql.run(
        GBootstrapReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
      );
      batch(() {
        viewer.value = data.viewer;
        serverName.value = data.serverInfo.name;
        serverVersion.value = data.serverInfo.version;
        serverBackend.value = data.serverInfo.backend;
        libraries.value = data.libraries.toList();
        categories.value = {
          for (final row in data.contentCategories) row.category: row.workCount,
        };
        linkedServers.value = data.linkedServers.toList();
        showLibraries.value = _preference(data, showLibrariesKey) == true;
        playbackPrefs.value = PlaybackPrefs.fromJson(
          _preference(data, 'playback'),
        );
        authError.value = '';
        phase.value = AppPhase.app;
      });
      unawaited(registerThisDevice());
    } on SessionExpired {
      // The expiry handler has already moved to the sign-in screen.
    } on ApiError catch (cause) {
      await tokens.clear();
      batch(() {
        viewer.value = null;
        authError.value = cause.message;
        phase.value = AppPhase.auth;
      });
    }
  }

  /// What a scan or a library change moved. Best effort, since what's on screen
  /// still works and the next launch catches up.
  Future<void> refreshLibraries() async {
    try {
      final data = await graphql.run(
        GBootstrapReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
      );
      batch(() {
        libraries.value = data.libraries.toList();
        categories.value = {
          for (final row in data.contentCategories) row.category: row.workCount,
        };
      });
    } on Exception {
      return;
    }
  }

  static Object? _preference(GBootstrapData data, String key) {
    for (final preference in data.preferences) {
      if (preference.key == key) {
        return (preference.value as JsonObject?)?.value;
      }
    }
    return null;
  }

  /// Tell the server what this device can play, so it can prepare versions it
  /// can and answer with its download settings. Best effort.
  Future<void> registerThisDevice() async {
    try {
      final profile = await decoders.ready();
      final data = await graphql.run(
        GRegisterDeviceReq(
          (b) => b.vars
            ..id = identity.id
            ..name = identity.name
            ..capabilities.replace(buildCapabilities(profile)),
        ),
      );
      device.value = data.registerDevice;
    } on Exception {
      // Retried on the next sign-in.
    }
  }

  /// Point the app at a server. The session belongs to the old one.
  Future<void> connect(String origin) async {
    if (origin != tokens.origin.value) {
      await tokens.clear();
      graphql.reset();
    }
    await tokens.setOrigin(origin);
    await start();
  }

  Future<void> signIn({required String access, required String refresh}) async {
    await tokens.save(access: access, refresh: refresh);
    _dropClientState();
    await bootstrap();
  }

  Future<void> login(String username, String password) async {
    final data = await graphql.run(
      GLoginReq(
        (b) => b.vars
          ..username = username
          ..password = password
          ..client = identity.clientName
          ..device = identity.name,
      ),
    );
    await signIn(access: data.login.token, refresh: data.login.refreshToken);
  }

  Future<void> completeSetup(String username, String password) async {
    final data = await graphql.run(
      GCompleteSetupReq(
        (b) => b.vars
          ..username = username
          ..password = password,
      ),
    );
    await signIn(
      access: data.completeSetupWizard.accessToken,
      refresh: data.completeSetupWizard.refreshToken,
    );
  }

  Future<void> redeemInvite(
    String token,
    String username,
    String password,
  ) async {
    final data = await graphql.run(
      GRedeemInviteReq(
        (b) => b.vars
          ..token = token
          ..username = username
          ..password = password
          ..client = identity.clientName
          ..device = identity.name,
      ),
    );
    await signIn(
      access: data.redeemInvite.token,
      refresh: data.redeemInvite.refreshToken,
    );
  }

  /// Sign out on purpose. Downloads go too: they're this account's, and the next
  /// person signing in on this device shouldn't find them. An expired session
  /// keeps them, as nobody chose to leave.
  Future<void> logout() async {
    try {
      await graphql.run(GLogoutReq());
    } on Exception {
      // Signing out locally regardless.
    }
    await getIt<DownloadsService>().removeAll();
    await tokens.clear();
    _signedOut('');
  }

  /// Forget this server altogether, to choose another.
  Future<void> forgetServer() async {
    await getIt<DownloadsService>().removeAll();
    await tokens.clear();
    await tokens.setOrigin(null);
    _signedOut('');
    phase.value = AppPhase.connect;
  }

  /// The clients go, and with them anything still listening to one.
  void _dropClientState() {
    graphql.reset();
    watchState.clear();
    getIt.resetLazySingleton<HomeService>();
  }

  void _signedOut(String message) {
    _dropClientState();
    batch(() {
      viewer.value = null;
      libraries.value = const [];
      categories.value = const {};
      linkedServers.value = const [];
      device.value = null;
      authError.value = message;
      phase.value = AppPhase.auth;
      getIt<NavService>().tab.value = ShellTab.home;
    });
  }
}
