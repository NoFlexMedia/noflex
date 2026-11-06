// SPDX-License-Identifier: AGPL-3.0-only
//
// The account page's state. Tab roots are all built at startup, so nothing is
// asked of the server until someone opens this tab, and it asks again each time
// they come back: sessions and exports change from elsewhere.

import 'dart:async';
import 'dart:convert';

import 'package:built_value/json_object.dart';
import 'package:ferry/ferry.dart';
import 'package:flutter/widgets.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/account.data.gql.dart';
import '../../graphql/queries/__generated__/account.req.gql.dart';
import '../../graphql/queries/__generated__/account.var.gql.dart';
import '../../graphql/queries/__generated__/authorize_device_pin.req.gql.dart';
import '../../graphql/queries/__generated__/cancel_export_job.req.gql.dart';
import '../../graphql/queries/__generated__/change_password.req.gql.dart';
import '../../graphql/queries/__generated__/create_api_key.req.gql.dart';
import '../../graphql/queries/__generated__/create_device_pin.req.gql.dart';
import '../../graphql/queries/__generated__/delete_export_job.req.gql.dart';
import '../../graphql/queries/__generated__/export_jobs.req.gql.dart';
import '../../graphql/queries/__generated__/linked_servers.req.gql.dart';
import '../../graphql/queries/__generated__/play_history.data.gql.dart';
import '../../graphql/queries/__generated__/play_history.req.gql.dart';
import '../../graphql/queries/__generated__/remove_device.req.gql.dart';
import '../../graphql/queries/__generated__/poll_device_pin.req.gql.dart';
import '../../graphql/queries/__generated__/remove_linked_server.req.gql.dart';
import '../../graphql/queries/__generated__/revoke_api_key.req.gql.dart';
import '../../graphql/queries/__generated__/revoke_session.req.gql.dart';
import '../../graphql/queries/__generated__/my_smart_marks.req.gql.dart';
import '../../graphql/queries/__generated__/prepared_versions.data.gql.dart';
import '../../graphql/queries/__generated__/prepared_versions.req.gql.dart';
import '../../graphql/queries/__generated__/set_preference.req.gql.dart';
import '../../graphql/queries/__generated__/unmark_for_smart_transcoding.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/smart_mark_fields.data.gql.dart';
import '../../graphql/queries/__generated__/update_device.req.gql.dart';
import '../../graphql/queries/__generated__/update_linked_server.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/export_job_fields.data.gql.dart';
import '../../graphql/query.dart';
import '../../models/work.dart';
import '../../services/device_identity.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
import '../../utils/tracks.dart';
import '../home/home_service.dart';
import '../player/decoder_probe.dart';
import '../player/decoders.dart';

typedef AccountSession = GAccountData_mySessions;
typedef AccountDevice = GAccountData_myDevices;
typedef ExportJob = GExportJobFields;
typedef AccountApiKey = GAccountData_apiKeys;
typedef HistoryRow = GPlayHistoryData_playHistory_nodes;

enum AccountSection {
  profile('Profile', PhosphorIconsRegular.user),
  devices('Devices', PhosphorIconsRegular.devices),
  servers('Servers', PhosphorIconsRegular.hardDrives),
  history('History', PhosphorIconsRegular.clockCounterClockwise),
  exports('Offline copies', PhosphorIconsRegular.export),
  smart('Smart transcoding', PhosphorIconsRegular.sparkle);

  const AccountSection(this.label, this.icon);
  final String label;
  final IconData icon;
}

const _exportPoll = Duration(seconds: 3);
const _marksPoll = Duration(seconds: 5);
const _activeExports = {
  GExportJobStatus.QUEUED,
  GExportJobStatus.RUNNING,
  GExportJobStatus.CANCEL_REQUESTED,
};

/// An export and the linked server it lives on; null for the home server.
class ExportEntry {
  const ExportEntry(this.job, [this.server]);
  final ExportJob job;
  final ServerRef? server;
}

bool isActiveExport(ExportJob job) => _activeExports.contains(job.status);

/// The `playback` preference with these choices written in. Other clients keep
/// their own fields in it, the web's `quality` and `autoplayNext` for two, so
/// it's merged rather than replaced.
Map<String, dynamic> mergePlaybackPreference(
  Object? stored,
  PlaybackPrefs prefs,
) {
  Map<String, dynamic> mapOf(Object? value) =>
      value is Map ? value.cast<String, dynamic>() : const {};
  final base = mapOf(stored);
  final chosen = prefs.toJson();
  return {
    ...base,
    'audio': {...mapOf(base['audio']), ...mapOf(chosen['audio'])},
    'subtitles': {...mapOf(base['subtitles']), ...mapOf(chosen['subtitles'])},
  };
}

/// A language field's value: comma or newline separated, in preference order.
List<String> splitLanguages(String value) => value
    .split(RegExp(r'[\n,]'))
    .map((part) => part.trim())
    .where((part) => part.isNotEmpty)
    .toList();

/// The auth session a token belongs to, which is how this one's found in the
/// list of them. It's only read for display, so an unreadable token is just no
/// session.
String? sessionIdOfToken(String? token) {
  final parts = token?.split('.');
  if (parts == null || parts.length < 2) return null;
  try {
    final claims = jsonDecode(
      utf8.decode(base64Url.decode(base64Url.normalize(parts[1]))),
    );
    return claims is Map && claims['sid'] is String
        ? claims['sid'] as String
        : null;
  } on FormatException {
    return null;
  }
}

String _pictureTier(int pixels) => pixels >= pixels2160p ? '4K' : '1080p';

/// What the probe found for a codec, in words.
({bool hardware, String detail}) decoderSummary(
  String codec,
  DecoderProfile profile,
) {
  final ceilings = profile.hardware[codec] ?? const <DecodeCeiling>[];
  if (ceilings.isEmpty) {
    return (
      hardware: false,
      detail: 'Software, up to ${_pictureTier(profile.softwareCeiling.pixels)}',
    );
  }
  int? most(bool Function(DecodeCeiling) where) {
    final pixels = ceilings.where(where).map((c) => c.pixels);
    return pixels.isEmpty ? null : pixels.reduce((a, b) => a > b ? a : b);
  }

  final any = most((_) => true)!;
  final tenBit = most((c) => c.bitDepth >= 10);
  final detail = tenBit == null
      ? '${_pictureTier(any)}, 8-bit'
      : tenBit == any
      ? '${_pictureTier(any)}, 10-bit'
      : '${_pictureTier(any)} 8-bit, ${_pictureTier(tenBit)} 10-bit';
  return (hardware: true, detail: 'Hardware, $detail');
}

class AccountController {
  Timer? _marksTimer;
  late final void Function() _stopFollowingMarks;

  AccountController() {
    _stopFollowingPrefs = effect(() {
      // Another client may have changed these since this app started.
      if (storedPlaybackJson.value == null) return;
      untracked(
        () => _session.playbackPrefs.value = PlaybackPrefs.fromJson(
          storedPlayback.value,
        ),
      );
    });
    _stopFollowingMarks = effect(() {
      final polling =
          section.value == AccountSection.smart &&
          getIt<NavService>().tab.value == ShellTab.account &&
          smartMarks.value.any(
            (mark) =>
                mark.state == GSmartTranscodeState.WAITING ||
                mark.state == GSmartTranscodeState.PREPARING,
          );
      untracked(() {
        _marksTimer?.cancel();
        _marksTimer = polling
            ? Timer.periodic(_marksPoll, (_) {
                unawaited(loadSmartMarks().catchError((_) {}));
              })
            : null;
      });
    });
    _stopFollowingExports = effect(() {
      final polling =
          hasActiveExports.value &&
          section.value == AccountSection.exports &&
          getIt<NavService>().tab.value == ShellTab.account;
      untracked(() {
        _poll?.cancel();
        _poll = polling
            ? Timer.periodic(_exportPoll, (_) {
                _query.value?.refetch();
                unawaited(loadLinkedExports());
              })
            : null;
      });
    });
  }

  GraphqlService get _graphql => getIt<GraphqlService>();
  ToastService get _toasts => getIt<ToastService>();
  SessionService get _session => getIt<SessionService>();

  final section = signal(AccountSection.profile);
  final remeasuring = signal(false);
  final _query = signal<Query<GAccountData, GAccountVars>?>(null);
  Timer? _poll;
  late final void Function() _stopFollowingPrefs;
  late final void Function() _stopFollowingExports;

  late final history = Pager<HistoryRow>(_historyPage);

  /// Whether this tab has been opened yet.
  late final opened = computed(() => _query.value != null);

  /// Only read once [opened].
  Query<GAccountData, GAccountVars> get query => _query.value!;

  late final _data = computed(() => _query.value?.data.value);

  late final currentSessionId = computed(
    () => sessionIdOfToken(getIt<TokenStore>().accessToken),
  );

  /// This one first, then whichever was seen most recently.
  late final sessions = computed<List<AccountSession>>(() {
    final current = currentSessionId.value;
    final nodes = _data.value?.mySessions;
    if (nodes == null) return const [];
    return nodes.toList()..sort((a, b) {
      final byCurrent = (b.id == current ? 1 : 0).compareTo(
        a.id == current ? 1 : 0,
      );
      return byCurrent != 0 ? byCurrent : b.lastSeenAt.compareTo(a.lastSeenAt);
    });
  });

  late final devices = computed<List<AccountDevice>>(
    () => _data.value?.myDevices.toList() ?? const [],
  );

  late final apiKeys = computed<List<AccountApiKey>>(
    () => [
      for (final key in _data.value?.apiKeys ?? const <AccountApiKey>[])
        if (key.revokedAt == null) key,
    ],
  );

  late final revokedApiKeys = computed(
    () => (_data.value?.apiKeys.length ?? 0) - apiKeys.value.length,
  );

  final _linkedExports = signal<List<ExportEntry>>(const []);

  late final exportJobs = computed<List<ExportEntry>>(() {
    final nodes = _data.value?.exportJobs.nodes;
    return [
      for (final ExportJob job in nodes ?? const <ExportJob>[])
        ExportEntry(job),
      ..._linkedExports.value,
    ]..sort((a, b) => b.job.createdAt.compareTo(a.job.createdAt));
  });

  /// A server that doesn't answer just has nothing to show for now.
  /// What the viewer has marked, the next to be prepared first.
  final smartMarks = signal<List<GSmartMarkFields>>(const []);

  final preparedVersions = signal<List<GPreparedVersionsData_preparedVersions>>(
    const [],
  );

  Future<void> loadSmartMarks() async {
    final data = await _graphql.run(
      GMySmartMarksReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
    );
    final prepared = await _graphql.run(
      GPreparedVersionsReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
    );
    batch(() {
      smartMarks.value = data.mySmartTranscodeMarks.toList();
      preparedVersions.value = prepared.preparedVersions.toList();
    });
  }

  Future<void> unmark(GSmartMarkFields mark) async {
    final done = await _toasts.attempt(
      () => _graphql.run(
        GUnmarkForSmartTranscodingReq(
          (b) => b.vars
            ..work = mark.work.id
            ..device = mark.device?.id,
        ),
      ),
      success: 'Unmarked',
    );
    if (done != null) await loadSmartMarks();
  }

  Future<void> loadLinkedExports() async {
    final servers = getIt<SessionService>().connectedServers.peek();
    final answers = await Future.wait(
      servers.map((server) async {
        try {
          final data = await _graphql.run(
            GExportJobsReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
            server: server,
          );
          return [
            for (final job in data.exportJobs.nodes) ExportEntry(job, server),
          ];
        } on Exception {
          return const <ExportEntry>[];
        }
      }),
    );
    _linkedExports.value = answers.expand((list) => list).toList();
  }

  late final hasActiveExports = computed(
    () =>
        exportJobs.value.any((ExportEntry entry) => isActiveExport(entry.job)),
  );

  late final streamCapBps = computed(
    () => _data.value?.myRestrictions.maxStreamBitrateBps,
  );

  /// The stored `playback` preference, whatever shape another client left it in.
  late final storedPlayback = computed<Object?>(() {
    for (final preference
        in _data.value?.preferences ?? const <GAccountData_preferences>[]) {
      if (preference.key == 'playback') return preference.value.value;
    }
    return null;
  });

  /// Changes only when the stored value does, unlike a freshly decoded map, so
  /// a form seeded from it isn't reset by every refetch. Null before it's known.
  late final storedPlaybackJson = computed<String?>(
    () => _data.value == null ? null : jsonEncode(storedPlayback.value),
  );

  void visit() {
    final existing = _query.value;
    if (existing == null) {
      _query.value = Query(
        _graphql.client,
        GAccountReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
      );
    } else {
      existing.refetch();
    }
    if (section.value == AccountSection.history) {
      unawaited(history.refresh());
    }
  }

  void select(AccountSection next) {
    section.value = next;
    switch (next) {
      case AccountSection.history:
        unawaited(history.refresh());
      case AccountSection.smart:
        break;
      case AccountSection.devices || AccountSection.exports:
        query.refetch();
      case AccountSection.profile || AccountSection.servers:
        break;
    }
  }

  Future<(List<HistoryRow>, String?)> _historyPage(String? after) async {
    final data = await _graphql.run(
      GPlayHistoryReq(
        (b) => b
          ..fetchPolicy = FetchPolicy.NetworkOnly
          ..vars.after = after,
      ),
    );
    final info = data.playHistory.pageInfo;
    return (
      data.playHistory.nodes.toList(),
      info.hasNextPage ? info.endCursor : null,
    );
  }

  // ---- profile ----------------------------------------------------------

  Future<bool> changePassword(String current, String next) async {
    final data = await _toasts.attempt(
      () => _graphql.run(
        GChangePasswordReq(
          (b) => b.vars
            ..current = current
            ..next = next,
        ),
      ),
      success: 'Password changed. Other devices will need to sign in again.',
    );
    if (data == null) return false;
    query.refetch();
    return true;
  }

  Future<bool> savePlayback(PlaybackPrefs prefs) async {
    final value = mergePlaybackPreference(storedPlayback.value, prefs);
    final data = await _toasts.attempt(
      () => _graphql.run(
        GSetPreferenceReq(
          (b) => b.vars
            ..key = 'playback'
            ..value = JsonObject(value),
        ),
      ),
      success: 'Playback preferences saved',
    );
    if (data == null) return false;
    _session.playbackPrefs.value = prefs;
    query.refetch();
    return true;
  }

  Future<void> setShowLibraries(bool show) async {
    final data = await _toasts.attempt(
      () => _graphql.run(
        GSetPreferenceReq(
          (b) => b.vars
            ..key = showLibrariesKey
            ..value = JsonObject(show),
        ),
      ),
    );
    if (data != null) _session.showLibraries.value = show;
  }

  /// Measure again, and tell the server, which prepares versions by what this
  /// device reports it can play.
  Future<void> remeasureDecoders() async {
    if (remeasuring.value) return;
    remeasuring.value = true;
    try {
      await getIt<DecoderProbeService>().remeasure();
      await _session.registerThisDevice();
      _toasts.show('Measured again', kind: ToastKind.success);
    } finally {
      remeasuring.value = false;
    }
  }

  // ---- devices ----------------------------------------------------------

  Future<void> revokeSession(String id) async {
    final data = await _toasts.attempt(
      () => _graphql.run(GRevokeSessionReq((b) => b.vars.id = id)),
      success: 'Device signed out',
    );
    if (data != null) query.refetch();
  }

  Future<void> revokeOtherSessions() async {
    final current = currentSessionId.value;
    final others = [
      for (final session in sessions.value)
        if (session.id != current) session.id,
    ];
    var failed = 0;
    await Future.wait([
      for (final id in others)
        _graphql
            .run(GRevokeSessionReq((b) => b.vars.id = id))
            .then<void>((_) {})
            .catchError((Object _) => failed++),
    ]);
    if (failed > 0) {
      _toasts.show(
        '$failed ${failed == 1 ? 'device' : 'devices'} couldn’t be signed out',
        kind: ToastKind.error,
      );
    } else {
      _toasts.show('Other devices signed out', kind: ToastKind.success);
    }
    query.refetch();
  }

  Future<void> forgetDevice(String id) async {
    final data = await _toasts.attempt(
      () => _graphql.run(GRemoveDeviceReq((b) => b.vars.id = id)),
      success: 'Device forgotten',
    );
    if (data != null) query.refetch();
  }

  Future<bool> approvePin(String pin) async {
    final data = await _toasts.attempt(() async {
      final result = await _graphql.run(
        // Codes are capitals, and the server matches them exactly.
        GAuthorizeDevicePinReq((b) => b.vars.pin = pin.toUpperCase()),
      );
      // A wrong code is an answer rather than an error, to the server.
      if (!result.authorizeDevicePin) {
        throw const ApiError('That code is wrong or has expired.');
      }
      return result;
    }, success: 'Device approved. It will finish signing in on its own.');
    if (data == null) return false;
    query.refetch();
    return true;
  }

  bool isThisDevice(String id) => id == getIt<DeviceIdentity>().id;

  Future<void> setSmartTranscode(AccountDevice device, bool enabled) async {
    final data = await _toasts.attempt(
      () => _graphql.run(
        GUpdateDeviceReq(
          (b) => b.vars
            ..id = device.id
            ..smartTranscode = enabled,
        ),
      ),
    );
    if (data != null) query.refetch();
  }

  /// Null lifts the limit.
  Future<void> setQualityLimit(AccountDevice device, int? maxBitrateBps) async {
    final data = await _toasts.attempt(
      () => _graphql.run(
        GUpdateDeviceReq(
          (b) => b.vars
            ..id = device.id
            ..maxBitrateBps = maxBitrateBps
            ..clearMaxBitrate = maxBitrateBps == null,
        ),
      ),
    );
    if (data != null) query.refetch();
  }

  /// The key itself, which the server keeps only a hash of.
  Future<String?> createApiKey(String name) async {
    final data = await _toasts.attempt(
      () => _graphql.run(GCreateApiKeyReq((b) => b.vars.name = name)),
    );
    if (data == null) return null;
    query.refetch();
    return data.createApiKey.key;
  }

  Future<void> revokeApiKey(String id) async {
    final data = await _toasts.attempt(
      () => _graphql.run(GRevokeApiKeyReq((b) => b.vars.id = id)),
      success: 'API key revoked',
    );
    if (data != null) query.refetch();
  }

  /// A code for another signed-in device to approve.
  Future<String?> generatePin() async {
    final identity = getIt<DeviceIdentity>();
    final data = await _toasts.attempt(
      () => _graphql.run(
        GCreateDevicePinReq(
          (b) => b.vars
            ..client = identity.clientName
            ..device = identity.name,
        ),
      ),
    );
    return data?.createDevicePin.pin;
  }

  /// Whether the code has been approved; asking again after that fails.
  Future<bool> pinApproved(String pin) async {
    final data = await _graphql.run(GPollDevicePinReq((b) => b.vars.pin = pin));
    if (data.pollDevicePin == null) return false;
    query.refetch();
    return true;
  }

  // ---- servers ----------------------------------------------------------

  /// The session holds these for everything that asks other servers; home and
  /// search pick up the change from there.
  Future<void> reloadLinkedServers() async {
    final data = await _graphql.run(
      GLinkedServersReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
    );
    _session.linkedServers.value = data.linkedServers.toList();
    getIt<HomeService>().refresh();
  }

  Future<void> setSync(String id, {required bool enabled}) async {
    await _toasts.attempt(() async {
      await _graphql.run(
        GUpdateLinkedServerReq(
          (b) => b.vars
            ..id = id
            ..sync = enabled,
        ),
      );
      await reloadLinkedServers();
    });
  }

  Future<void> removeServer(String id) async {
    await _toasts.attempt(() async {
      await _graphql.run(GRemoveLinkedServerReq((b) => b.vars.id = id));
      await reloadLinkedServers();
    }, success: 'Server removed');
  }

  // ---- exports ----------------------------------------------------------

  Future<void> removeExport(ExportEntry entry) async {
    final data = await _toasts.attempt(
      () => _graphql.run(
        GDeleteExportJobReq((b) => b.vars.id = entry.job.id),
        server: entry.server,
      ),
      success: 'Offline copy removed',
    );
    if (data == null) return;
    query.refetch();
    await loadLinkedExports();
  }

  Future<void> cancelExport(ExportEntry entry) async {
    final data = await _toasts.attempt(
      () => _graphql.run(
        GCancelExportJobReq((b) => b.vars.id = entry.job.id),
        server: entry.server,
      ),
      success: 'Offline copy cancelled',
    );
    if (data == null) return;
    query.refetch();
    await loadLinkedExports();
  }

  void dispose() {
    _marksTimer?.cancel();
    _stopFollowingMarks();
    _poll?.cancel();
    _stopFollowingPrefs();
    _stopFollowingExports();
    _query.value?.dispose();
  }
}
