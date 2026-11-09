// SPDX-License-Identifier: AGPL-3.0-only
//
// The app's lifecycle, a port of web/src/lib/session.svelte.ts with the phases
// a native client needs on top: choosing a server, there being no same-origin,
// running offline from what's been downloaded, and choosing who's watching, as
// a device several people use keeps a session for each of them.

import 'dart:async';

import 'package:built_value/json_object.dart';
import 'package:flutter/foundation.dart';
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
import '../graphql/queries/__generated__/remembered_profiles.req.gql.dart';
import '../graphql/queries/fragments/__generated__/device_fields.data.gql.dart';
import '../graphql/queries/fragments/__generated__/library_fields.data.gql.dart';
import '../graphql/queries/fragments/__generated__/linked_server_fields.data.gql.dart';
import '../graphql/queries/fragments/__generated__/user_fields.data.gql.dart';
import '../models/work.dart';
import '../utils/tracks.dart';
import 'device_identity.dart';
import 'graphql.dart';
import 'indexing_service.dart';
import 'nav_service.dart';
import 'tokens.dart';
import 'watch_state.dart';

enum AppPhase { boot, connect, error, setup, profiles, auth, app, offline }

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
    tokens.onExpired = (error, account) =>
        unawaited(_afterLeaving(_endedMessage(account?.name, error)));
    // Locked from elsewhere, or a PIN set since this device last asked.
    tokens.onLocked = () {
      final userId = tokens.activeId.value;
      if (userId == null) return;
      unawaited(
        tokens
            .describe(userId, hasPin: true)
            .then((_) => _showProfiles(message: 'Enter your PIN to carry on.')),
      );
    };
  }

  /// Why someone has to sign in again, naming them: with several tiles, "your
  /// session" doesn't say whose.
  static String _endedMessage(String? name, SessionExpired error) {
    if (name == null) return 'Your session expired. Sign in again to continue.';
    return error.reused
        ? '$name was signed out because their session was used from somewhere '
              'else. Sign in again, and change the password if that wasn’t them.'
        : '$name’s session expired. Sign in again to continue.';
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

  /// What went wrong on the way in, for the sign-in screen and "Who's watching?".
  final authError = signal('');

  /// Whose session the sign-in screen is bringing back: someone still on this
  /// device whose session ended. Null when it's signing in somebody new.
  final signingInFor = signal<String?>(null);

  /// Whether the server answered at launch. Without it a profile can still be
  /// chosen for what it has downloaded, but not one behind a PIN: only the
  /// server can check that.
  final reachable = signal(true);

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

  DownloadsService get _downloads => getIt<DownloadsService>();

  /// One person, still in their profile, goes straight back to it. Anyone else
  /// starts from "Who's watching?": several people share the device, or the
  /// one who does stepped away from their profile on purpose.
  bool get _resumesAlone =>
      tokens.hasSession && tokens.accounts.value.length == 1;

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
      batch(() {
        serverName.value = info.name;
        reachable.value = true;
      });
      if (!info.isConfigured) {
        phase.value = AppPhase.setup;
        return;
      }
      if (tokens.accounts.value.isEmpty) {
        phase.value = AppPhase.auth;
      } else if (_resumesAlone) {
        await _useDownloads(tokens.activeId.value);
        await bootstrap();
      } else {
        await _showProfiles();
      }
    } on ServerUnreachable catch (cause) {
      reachable.value = false;
      // Not a sign-out, and what's downloaded still plays.
      if (_resumesAlone) {
        await _useDownloads(tokens.activeId.value);
        phase.value = AppPhase.offline;
      } else if (tokens.accounts.value.isNotEmpty) {
        await _showProfiles();
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
      // Their tile next time, as the server knows them now. The picture comes
      // from "Who's watching?" itself: this one's URL dies with a lock.
      await tokens.describe(
        data.viewer.id,
        name: data.viewer.name,
        hasPin: data.viewer.hasPin,
        needsSignIn: false,
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
      // The expiry and lock handlers have already moved on from here.
    } on ApiError catch (cause) {
      // Refused as they are, which signing in again may mend. They stay.
      final userId = tokens.activeId.value;
      if (userId != null) await tokens.end(userId);
      await _afterLeaving(cause.message);
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
            ..client = identity.clientName
            ..capabilities.replace(buildCapabilities(profile)),
        ),
      );
      device.value = data.registerDevice;
    } on Exception {
      // Retried on the next sign-in.
    }
  }

  /// Point the app at a server. The sessions belong to the old one.
  Future<void> connect(String origin) async {
    if (origin != tokens.origin.value) {
      await tokens.clear();
      graphql.reset();
    }
    await tokens.setOrigin(origin);
    await start();
  }

  /// Adopt a session the server just issued and go in as its user. Someone
  /// already signed in here has their old session replaced, not a second tile.
  Future<void> signIn({
    required String userId,
    required String name,
    required String access,
    required String refresh,
    bool hasPin = false,
  }) async {
    await tokens.signIn(
      StoredAccount(
        userId: userId,
        name: name,
        access: access,
        refresh: refresh,
        hasPin: hasPin,
        avatarUrl: tokens.account(userId)?.avatarUrl,
      ),
    );
    signingInFor.value = null;
    await _enter();
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
    await signIn(
      userId: data.login.user.id,
      name: data.login.user.name,
      hasPin: data.login.user.hasPin,
      access: data.login.token,
      refresh: data.login.refreshToken,
    );
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
      userId: data.completeSetupWizard.userId,
      name: username,
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
      userId: data.redeemInvite.user.id,
      name: data.redeemInvite.user.name,
      hasPin: data.redeemInvite.user.hasPin,
      access: data.redeemInvite.token,
      refresh: data.redeemInvite.refreshToken,
    );
  }

  // ---- who's watching ---------------------------------------------------

  /// Go in as someone signed in here. A profile with a PIN opens only through
  /// the server, whether or not its session got as far as being locked.
  ///
  /// Throws [WrongPin] while there are tries left; [ApiError] once there aren't
  /// or the session turns out to be gone, by which time their tile says to sign
  /// in again and [authError] why; and [ServerUnreachable].
  Future<void> chooseProfile(String userId, {String pin = ''}) async {
    final account = tokens.account(userId);
    if (account == null) return;
    if (account.hasPin) {
      try {
        await tokens.unlock(userId, pin);
      } on ApiError catch (error) {
        await tokens.end(userId);
        final message = error.codes.contains('PIN_ATTEMPTS_EXHAUSTED')
            ? 'Too many wrong PINs. Sign in again to use this profile.'
            : '${account.name} was signed out here. Sign in again to continue.';
        // Said where they are now: beside the tiles, or on the sign-in screen
        // when theirs was the only one.
        if (_soleEnded != null) {
          _toSignIn(userId, message: message);
        } else {
          authError.value = message;
        }
        throw ApiError(message);
      }
    }
    await tokens.activate(userId);
    try {
      await _enter();
    } on ServerUnreachable {
      // Still on "Who's watching?", where nobody is active.
      await tokens.deactivate();
      rethrow;
    }
  }

  /// Step away from the active profile, to "Who's watching?".
  Future<void> switchProfile() => _showProfiles();

  /// Sign someone new in.
  void addProfile() => _toSignIn(null);

  /// Sign in again for someone whose session ended, to bring their profile back.
  void signInAgain(String userId) => _toSignIn(userId);

  void _toSignIn(String? userId, {String message = ''}) => batch(() {
    signingInFor.value = userId;
    authError.value = message;
    phase.value = AppPhase.auth;
  });

  /// Back from signing someone in, with nobody new.
  void cancelAddProfile() {
    if (tokens.accounts.value.isEmpty) return;
    batch(() {
      signingInFor.value = null;
      phase.value = AppPhase.profiles;
    });
  }

  /// Take someone off this device, with what they downloaded. For a session
  /// that ended and won't be signed in to again; someone signed in signs out.
  Future<void> removeProfile(String userId) async {
    await _guarded(() => _downloads.removeAccount(userId));
    await tokens.remove(userId);
    await _afterLeaving('');
  }

  /// One person, who has to sign in again: their tile would only be a step in
  /// the way. Signing in needs the server, so without it the tile stays.
  StoredAccount? get _soleEnded {
    final accounts = tokens.accounts.value;
    return reachable.value &&
            accounts.length == 1 &&
            accounts.single.needsSignIn
        ? accounts.single
        : null;
  }

  /// What the server says of everyone signed in here: their tiles as they are
  /// now, and which sessions it no longer knows. Best effort, as the tiles
  /// already show what was true last time.
  Future<void> refreshProfiles() async {
    final asked = tokens.accounts.value;
    if (asked.isEmpty) return;
    try {
      final data = await graphql.run(
        GRememberedProfilesReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NetworkOnly
            ..vars.tokens.addAll([
              for (final account in asked) account.refresh,
            ]),
        ),
      );
      for (final (index, profile) in data.rememberedProfiles.indexed) {
        final account = asked[index];
        // Rotated since, so the answer is about a token that's gone anyway.
        if (tokens.account(account.userId)?.refresh != account.refresh) {
          continue;
        }
        if (profile == null) {
          await tokens.describe(account.userId, needsSignIn: true);
        } else {
          await tokens.describe(
            account.userId,
            name: profile.name,
            hasPin: profile.hasPin,
            avatarUrl: profile.avatarUrl,
            clearAvatar: profile.avatarUrl == null,
            needsSignIn: false,
          );
        }
      }
    } on Exception {
      return;
    }
    // Found out here that the one person has to sign in again.
    final sole = _soleEnded;
    if (sole != null && phase.value == AppPhase.profiles) {
      _toSignIn(sole.userId, message: authError.value);
    }
  }

  /// The headless sync shares the session, and may have found it ended while
  /// the app was away: whoever was here is then nobody's active account, and
  /// needs to sign in again. Returns whether that happened, and moves on from
  /// the shell.
  Future<bool> lostWhileAway() async {
    if (phase.value != AppPhase.app || tokens.hasSession) return false;
    final ended = tokens.account(viewer.value?.id);
    await _afterLeaving(_endedMessage(ended?.name, const SessionExpired()));
    return true;
  }

  Future<void> _enter() async {
    _dropClientState();
    await _useDownloads(tokens.activeId.value);
    if (reachable.value) {
      await bootstrap();
    } else {
      phase.value = AppPhase.offline;
    }
  }

  /// Nobody is active from here. Whoever was is left behind their PIN if they
  /// have one, which is what makes switching away mean something.
  Future<void> _showProfiles({String message = ''}) async {
    final left = tokens.active;
    if (left != null) {
      if (left.hasPin && reachable.value) {
        // Theirs may not be loaded yet, when this is the app starting.
        await _useDownloads(left.userId);
        await _guarded(_downloads.hold);
        await tokens.lock(left.userId);
      }
      await tokens.deactivate();
    }
    await _useDownloads(null);
    final sole = _soleEnded;
    if (sole != null) {
      _signedOut(message);
      signingInFor.value = sole.userId;
      return;
    }
    _signedOut(message, to: AppPhase.profiles);
    unawaited(refreshProfiles());
  }

  /// Where to go once nobody is active: whoever is signed in here, if anyone.
  Future<void> _afterLeaving(String message) async {
    if (tokens.accounts.value.isEmpty) {
      await _useDownloads(null);
      _signedOut(message);
    } else {
      await _showProfiles(message: message);
    }
  }

  Future<void> _useDownloads(String? userId) =>
      _guarded(() => _downloads.useAccount(userId));

  /// Streaming needs none of the downloader, so a broken one mustn't keep
  /// anyone out.
  static Future<void> _guarded(Future<void> Function() action) async {
    try {
      await action();
    } on Object catch (error) {
      debugPrint('downloads unavailable: $error');
    }
  }

  /// Sign out on purpose. This account's downloads go too: they're theirs, and
  /// the next person on this device shouldn't find them. An expired session
  /// keeps them, and its tile, as nobody chose to leave. Anyone else signed in
  /// here stays.
  Future<void> logout() async {
    try {
      await graphql.run(GLogoutReq());
    } on Exception {
      // Signing out locally regardless.
    }
    await _guarded(_downloads.removeAll);
    await tokens.removeActive();
    await _afterLeaving('');
  }

  /// Forget this server altogether, to choose another: everyone signed in
  /// here, and what they downloaded.
  Future<void> forgetServer() async {
    await _guarded(_downloads.removeEveryone);
    await tokens.clear();
    await tokens.setOrigin(null);
    _signedOut('', to: AppPhase.connect);
  }

  /// The clients go, and with them anything still listening to one.
  void _dropClientState() {
    graphql.reset();
    watchState.clear();
    getIt.resetLazySingleton<HomeService>();
    getIt.resetLazySingleton<IndexingService>();
  }

  void _signedOut(String message, {AppPhase to = AppPhase.auth}) {
    _dropClientState();
    batch(() {
      viewer.value = null;
      libraries.value = const [];
      categories.value = const {};
      linkedServers.value = const [];
      device.value = null;
      authError.value = message;
      signingInFor.value = null;
      phase.value = to;
      getIt<NavService>().tab.value = ShellTab.home;
    });
  }
}
