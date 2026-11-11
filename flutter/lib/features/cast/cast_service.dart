// SPDX-License-Identifier: AGPL-3.0-only
//
// Google Cast from this app: a port of web/src/lib/cast.svelte.ts that talks to
// the device itself (cast_channel.dart) rather than through Google's sender SDK,
// which is closed source, so an AGPL app can't ship it, and exists only for
// Android and iOS. A Cast device found on the network is one more place in the
// Play-on list. Choosing it opens a session there, and from then on this
// resolves the title, opens the server's playback session with what the device
// decodes, resumes from the viewer's place and reports progress, for as long
// as the session lasts. The receiver (cast/receiver) only plays.

import 'dart:async';
import 'dart:math' as math;

import 'package:ferry/ferry.dart';
import 'package:flutter/widgets.dart';
import 'package:http/http.dart' as http;
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/mark_played.req.gql.dart';
import '../../graphql/queries/__generated__/media_files.req.gql.dart';
import '../../graphql/queries/__generated__/report_progress.req.gql.dart';
import '../../graphql/queries/__generated__/work_detail.req.gql.dart';
import '../../graphql/queries/__generated__/works_by_id.data.gql.dart';
import '../../graphql/queries/__generated__/works_by_id.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/media_file_full.data.gql.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
import '../../services/watch_state.dart';
import '../../utils/images.dart';
import '../../utils/tracks.dart';
import '../connect/connect_service.dart';
import '../player/playback_service.dart';
import '../player/quality.dart';
import 'cast_channel.dart';
import 'cast_discovery.dart';
import 'cast_profile.dart';

const _defaultReceiver = 'CC1AD845';

/// Google's Default Media Receiver until a NoFlex receiver's id is built in.
const _appId = String.fromEnvironment(
  'NOFLEX_CAST_APP_ID',
  defaultValue: _defaultReceiver,
);
const _receiverNamespace = 'urn:x-cast:com.google.cast.receiver';
const _mediaNamespace = 'urn:x-cast:com.google.cast.media';

/// Where NoFlex's receiver answers what its device decodes (cast/receiver).
const _noflexNamespace = 'urn:x-cast:dev.noflex.cast';

/// The dialect `cast/receiver` is told this sender speaks.
const _protocol = 1;
const _reportEvery = Duration(seconds: 15);

/// How often to ask where playback is: a device only says when its state changes.
const _statusEvery = Duration(seconds: 5);

// A linear transcode's seek past what it has produced restarts it at the target, as the player does.
const _transcodeReach = 30;

/// How long to wait before each try at finding a device again after its
/// connection dropped: about two minutes in all, as a phone off the Wi-Fi
/// for a moment or a device restarting its network comes back within it.
const _rejoinWaits = [1, 2, 4, 8, 15, 15, 15, 15, 15, 15];

const _plans = {
  'DIRECT_PLAY': 'Direct play',
  'REMUX': 'Direct stream (remux)',
  'TRANSCODE_AUDIO': 'Transcoding audio',
  'TRANSCODE_VIDEO': 'Transcoding video',
  'TRANSCODE': 'Transcoding',
};

/// A Cast device can't reach this device's own loopback address.
bool _loopback(String origin) {
  final host = Uri.tryParse(origin)?.host ?? '';
  return host == 'localhost' || host.startsWith('127.') || host == '::1';
}

/// A session on one Cast device: the connection, and the app it runs for us.
class _Session {
  _Session(this.device, this.channel, this.transportId, this.sessionId);
  final CastReceiverInfo device;
  final CastChannel channel;
  final String transportId;
  final String sessionId;
  StreamSubscription<CastEvent>? events;

  /// This app launched the receiver or played on it, so ending the session
  /// stops it; one only joined is left as it was.
  bool owned = false;
}

/// The media session's last status, kept through a dropped connection.
class _Media {
  int? id;
  String playerState = 'IDLE';
  double currentTime = 0;
  bool awaitingPosition = true;
  double? duration;
  DateTime heardAt = DateTime.now();

  /// Seconds into the stream now, advanced since the last status while playing.
  double get now => playerState == 'PLAYING'
      ? currentTime + DateTime.now().difference(heardAt).inMilliseconds / 1000
      : currentTime;
}

/// The device is reachable but isn't running the receiver: nothing of ours
/// plays there.
class _NotRunning implements Exception {
  const _NotRunning();
}

/// What this sender loaded, which the Cast device doesn't know.
class _Loaded {
  const _Loaded({
    required this.work,
    required this.mediaFileId,
    required this.editionId,
    required this.sessionId,
    required this.transcode,
    required this.offsetMs,
    required this.durationMs,
    required this.quality,
    required this.leftOriginal,
    required this.adaptive,
    required this.sourceBitrateBps,
  });

  final WorkSummary work;
  final String mediaFileId;
  final String editionId;
  final String sessionId;
  final bool transcode;

  /// Source time of the stream's zero.
  final int offsetMs;

  /// The real length, as a growing transcode doesn't know it.
  final int? durationMs;
  final QualityChoice quality;

  /// Auto moved this title off the original because it struggled.
  final bool leftOriginal;

  /// The stream is a ladder the receiver adapts along.
  final bool adaptive;

  /// The original file's rate, for Auto to weigh the connection against.
  final int? sourceBitrateBps;
}

typedef _Playable = ({
  GMediaFileFull file,
  String editionId,
  int? audio,
  int? subtitle,
  int? burn,
  int resume,
  int? runtimeMs,
});

class CastService implements RemoteRoute {
  CastService({
    required SessionService session,
    required this._connect,
    required this._playback,
    required this._graphql,
    required this._tokens,
    required this._toasts,
  }) {
    _connect.castRoute = this;
    _stopWatchingSession = effect(() {
      _signedIn = session.phase.value == AppPhase.app;
      untracked(_follow);
    });
    _stopWatchingDevices = effect(() {
      _discovery.devices.value;
      untracked(() {
        _publish();
        _peekAll();
      });
    });
    _stopFollowingTarget = effect(() {
      final id = _connect.targetId.value;
      untracked(() => _choose(id));
    });
    _lifecycle = AppLifecycleListener(
      onStateChange: (state) {
        _foreground = state == AppLifecycleState.resumed;
        // Back in front, a device may be playing what this app lost hold of
        // while away, which the system may have cut off.
        if (_foreground) _peeked.clear();
        _follow();
        _peekAll();
      },
    );
  }

  final ConnectService _connect;
  final PlaybackService _playback;
  final GraphqlService _graphql;
  final TokenStore _tokens;
  final ToastService _toasts;
  final _discovery = CastDiscovery();
  late final void Function() _stopWatchingSession;
  late final void Function() _stopWatchingDevices;
  late final void Function() _stopFollowingTarget;
  late final AppLifecycleListener _lifecycle;
  var _signedIn = false;
  var _foreground = true;

  /// The ladder the title on the device is offered at, for the quality menu.
  final qualities = signal<List<StreamQuality>>(const []);
  final choice = signal(QualityChoice.original);

  /// The receiver is showing its stream info panel.
  final statsOnTv = signal(false);

  /// Whether the session's device runs NoFlex's receiver, which can show stream info.
  bool get ownReceiver => _appId != _defaultReceiver;

  _Session? _session;
  Future<_Session>? _opening;
  _Loaded? _loaded;
  var _media = _Media();

  /// The device a title plays on while its connection is found again.
  CastReceiverInfo? _away;

  /// Devices looked at for a title this app cast before it last closed, since
  /// it came to the front.
  final _peeked = <String>{};

  /// A title being started, which the Cast device hasn't answered for yet.
  ({WorkSummary work, int positionMs})? _preparing;
  var _auto = AutoQuality();

  /// The receiver's latest report on playback.
  Map<String, dynamic>? _health;
  var _progressReportedAt = 0;
  var _switching = false;
  GPlaybackState? _last;
  Timer? _beat;
  Timer? _steer;
  Timer? _poll;

  /// What each device decodes, probed once while the app runs.
  final _profiles = <String, Future<GClientCapabilities>>{};

  String _id(CastReceiverInfo device) => 'cast:${device.id}';

  CastReceiverInfo? _infoFor(String id) {
    // The latest announcement first, as a device's address can change.
    for (final device in _discovery.devices.peek()) {
      if (_id(device) == id) return device;
    }
    for (final device in [?_session?.device, ?_away]) {
      if (_id(device) == id) return device;
    }
    return null;
  }

  /// Look for devices while someone is signed in and the app is in front;
  /// a session keeps going in the background.
  void _follow() {
    if (_signedIn && _foreground) {
      unawaited(_discovery.start());
    } else {
      unawaited(_discovery.stop());
    }
    if (!_signedIn) unawaited(_session != null ? _close() : _abandon());
  }

  /// The viewer chose where Play goes: open a session on a Cast device as it's
  /// chosen, so its screen is ready, and leave one that isn't playing when
  /// they choose elsewhere.
  void _choose(String? id) {
    final info = id == null ? null : _infoFor(id);
    if (info != null) {
      unawaited(
        _connectTo(info).then<void>(
          (_) {},
          onError: (Object error) {
            _toasts.show(
              'Couldn’t connect to ${info.name}.',
              kind: ToastKind.error,
            );
            if (_connect.targetId.peek() == id) _connect.setTarget(null);
          },
        ),
      );
    } else if (_session != null && _loaded == null && _preparing == null) {
      unawaited(_close());
    }
  }

  /// The session on [info], opened if need be. Without [launch], only a
  /// receiver already running there is joined, or [_NotRunning] thrown.
  Future<_Session> _connectTo(
    CastReceiverInfo info, {
    bool launch = true,
  }) async {
    final current = _session;
    if (current != null && current.device.id == info.id) return current;
    final opening = _opening;
    if (opening != null) {
      final session = await opening.then<_Session?>(
        (session) => session,
        onError: (Object _) => null,
      );
      if (session != null && session.device.id == info.id) return session;
    }
    if (_session != null) await _close();
    // Played elsewhere now: the device out of reach is let go.
    if (_away case final away? when away.id != info.id) await _abandon();
    return _opening ??= _open(
      info,
      launch: launch,
    ).whenComplete(() => _opening = null);
  }

  Future<_Session> _open(CastReceiverInfo info, {required bool launch}) async {
    final channel = await CastChannel.connect(info.host, info.port);
    try {
      ({String sessionId, String transportId})? appIn(CastEvent event) {
        final applications = event.data['status']?['applications'];
        if (applications is! List) return null;
        for (final app in applications.whereType<Map<String, dynamic>>()) {
          if (app['appId'] == _appId) {
            return (
              sessionId: app['sessionId'] as String,
              transportId: app['transportId'] as String,
            );
          }
        }
        return null;
      }

      // Join our app if it's already up there, as Google's SDK does.
      var app = appIn(
        await channel.request(_receiverNamespace, {'type': 'GET_STATUS'}),
      );
      final launching = app == null;
      if (launching) {
        if (!launch) throw const _NotRunning();
        final launched = channel.events
            .where((event) => appIn(event) != null)
            .first;
        final reply = await channel.request(_receiverNamespace, {
          'type': 'LAUNCH',
          'appId': _appId,
        }, timeout: const Duration(seconds: 20));
        if (reply.type == 'LAUNCH_ERROR') {
          throw StateError('launch refused: ${reply.data['reason']}');
        }
        app =
            appIn(reply) ??
            appIn(await launched.timeout(const Duration(seconds: 20)));
      }
      final session = _Session(info, channel, app!.transportId, app.sessionId)
        ..owned = launching;
      channel.open(session.transportId);
      session.events = channel.events.listen(
        (event) => _onEvent(session, event),
      );
      unawaited(channel.closed.then((_) => _lost(session)));
      _session = session;
      _away = null;
      await _resync(session);
      // Probe while the viewer picks a title, so loading doesn't wait on it.
      unawaited(_profile(session));
      _publish();
      return session;
    } on Object {
      await channel.close();
      rethrow;
    }
  }

  /// What the device plays as a connection opens: the title this app was
  /// following, carried on or over; one it cast before it last closed, to
  /// take up again; or nothing of ours.
  Future<void> _resync(_Session session) async {
    final CastEvent reply;
    try {
      reply = await session.channel.request(
        _mediaNamespace,
        {'type': 'GET_STATUS'},
        destination: session.transportId,
        timeout: const Duration(seconds: 5),
      );
    } on Object {
      // Its next status says, or the poll asks again.
      if (_session == session && _loaded != null) _startTimers();
      return;
    }
    if (_session != session) return;
    final statuses = reply.data['status'];
    final status = statuses is List
        ? statuses.whereType<Map<String, dynamic>>().firstOrNull
        : null;
    final playing = status != null && status['playerState'] != 'IDLE';
    final custom = status?['media']?['customData'];
    final sender = custom is Map<String, dynamic> ? custom['sender'] : null;
    final loaded = _loaded;
    if (loaded != null) {
      final same =
          playing &&
          (status['mediaSessionId'] == _media.id ||
              (sender is Map && sender['sessionId'] == loaded.sessionId));
      if (same) {
        session.owned = true;
        // Reported at once: the device went on without us.
        _last = null;
        _media.id = status['mediaSessionId'] as int?;
        _onMediaStatus(session, reply.data);
        _startTimers();
      } else {
        // It ended, or something else was put on, while we were away.
        await _finish(ended: status?['idleReason'] == 'FINISHED');
      }
    } else if (playing &&
        _preparing == null &&
        sender is Map<String, dynamic>) {
      await _adopt(session, status, custom as Map<String, dynamic>, sender);
    }
  }

  /// Take up a title this app cast before it closed or lost the device, from
  /// what the load request carried, so it's reported and steered again.
  Future<void> _adopt(
    _Session session,
    Map<String, dynamic> status,
    Map<String, dynamic> custom,
    Map<String, dynamic> sender,
  ) async {
    // Someone else's, cast from their own account.
    if (sender['user'] != _tokens.activeId.peek()) return;
    final serverId = sender['server'] as String?;
    final server = serverId == null
        ? null
        : getIt<SessionService>().connectedServers
              .peek()
              .where((server) => server.id == serverId)
              .firstOrNull;
    final workId = custom['workId'] as String?;
    if ((serverId != null && server == null) || workId == null) return;
    final GWorksByIdData found;
    try {
      found = await _graphql.run(
        GWorksByIdReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NetworkOnly
            ..vars.ids.add(workId),
        ),
        server: server,
      );
    } on Exception {
      return;
    }
    final card = found.worksById.firstOrNull;
    if (card == null || _session != session || _loaded != null) return;
    final stream = custom['stream'] as Map<String, dynamic>?;
    final quality = decodeChoice(sender['quality'] as String?);
    _progressReportedAt = 0;
    _media = _Media()..awaitingPosition = false;
    _loaded = _Loaded(
      work: WorkSummary.fromCard(card, server: server),
      mediaFileId: sender['mediaFileId'] as String,
      editionId: sender['editionId'] as String,
      sessionId: sender['sessionId'] as String,
      transcode: sender['transcode'] == true,
      offsetMs: (custom['offsetMs'] as num?)?.toInt() ?? 0,
      durationMs: (custom['durationMs'] as num?)?.toInt(),
      quality: quality,
      leftOriginal: sender['leftOriginal'] == true,
      adaptive: stream?['adaptive'] == true,
      sourceBitrateBps: (stream?['source']?['bitrateBps'] as num?)?.toInt(),
    );
    session.owned = true;
    _auto = AutoQuality()..settle();
    qualities.value = [
      for (final step
          in (sender['qualities'] as List? ?? const [])
              .whereType<Map<String, dynamic>>())
        StreamQuality(
          label: step['label'] as String,
          width: (step['width'] as num).toInt(),
          height: (step['height'] as num).toInt(),
          bitrateBps: (step['bitrateBps'] as num).toInt(),
        ),
    ];
    choice.value = quality;
    _onMediaStatus(session, {
      'status': [status],
    });
    _startTimers();
  }

  /// The connection went. What plays there carries on without it, and so does
  /// its stream on the server, so look for the device again rather than end it.
  void _lost(_Session session) {
    if (_session != session) return;
    _session = null;
    unawaited(session.events?.cancel());
    _poll?.cancel();
    if (_loaded != null) {
      _away = session.device;
      unawaited(_rejoin(session));
    }
    _publish();
  }

  Future<void> _rejoin(_Session previous) async {
    final loaded = _loaded;
    final id = _id(previous.device);
    for (final wait in _rejoinWaits) {
      await Future<void>.delayed(Duration(seconds: wait));
      if (_loaded != loaded || _session != null || !_signedIn) return;
      try {
        await _connectTo(_infoFor(id) ?? previous.device, launch: false);
        return;
      } on _NotRunning {
        // Reachable, and our receiver is gone: the title ended with it.
        _away = null;
        await _finish(ended: false);
        return;
      } on Object {
        // Not reachable yet.
      }
    }
    if (_loaded != loaded) return;
    // Found again later, it's taken up then (_peek).
    _peeked.remove(previous.device.id);
    await _abandon();
  }

  /// Stop following the title on a device out of reach. It may well play on,
  /// and the server keeps its stream while it's fetched, so only this app lets
  /// go, at the device's last word.
  Future<void> _abandon() async {
    _away = null;
    await _finish(ended: false, stopServer: false);
  }

  /// Look on [info] for a title this app cast before it closed, starting nothing.
  Future<void> _peek(CastReceiverInfo info) async {
    try {
      final session = await _connectTo(info, launch: false);
      if (_loaded == null &&
          _session == session &&
          _connect.targetId.peek() != _id(info)) {
        await _leave(session);
      }
    } on Object {
      // Not ours there, or out of reach.
    }
    _peekAll();
  }

  /// Peek at the devices found, one at a time, while nothing is held here.
  void _peekAll() {
    if (!_signedIn ||
        !_foreground ||
        _session != null ||
        _opening != null ||
        _loaded != null) {
      return;
    }
    for (final info in _discovery.devices.peek()) {
      if (_peeked.add(info.id)) {
        unawaited(_peek(info));
        return;
      }
    }
  }

  /// Let the device go without touching what it plays.
  Future<void> _leave(_Session session) async {
    _lost(session);
    await session.channel.close();
  }

  /// End the session: what plays there, the app on the device if it's ours to
  /// stop, and the connection to it.
  Future<void> _close() async {
    final session = _session;
    if (session == null) return;
    await _finish(ended: false);
    if (session.owned) {
      session.channel.send(_receiverNamespace, {
        'type': 'STOP',
        'sessionId': session.sessionId,
        'requestId': 0,
      });
    }
    await _leave(session);
  }

  /// The device ended our session, or something else took the screen.
  Future<void> _ended(_Session session) async {
    // The connection may have dropped on the way, leaving the title waiting on it.
    if (_session == session || _away?.id == session.device.id) {
      _away = null;
      await _finish(ended: false);
    }
    await _leave(session);
  }

  /// A media command whose reply is the status that follows anyway.
  void _command(_Session session, Map<String, Object?> payload) {
    unawaited(
      session.channel
          .request(_mediaNamespace, payload, destination: session.transportId)
          .then<void>((_) {}, onError: (Object _) {}),
    );
  }

  void _onEvent(_Session session, CastEvent event) {
    switch ((event.namespace, event.type)) {
      case (_mediaNamespace, 'MEDIA_STATUS'):
        _onMediaStatus(session, event.data);
      case (_noflexNamespace, 'health'):
        _onHealth(event.data);
      case (_noflexNamespace, 'progress') when _session == session:
        final now = DateTime.now().millisecondsSinceEpoch;
        final suppressed = _progressReportedAt != 0;
        final reporting = event.data['reporting'] == true;
        _progressReportedAt = reporting && _loaded != null ? now : 0;
        if (!reporting && suppressed && _media.playerState != 'IDLE') {
          unawaited(
            _report(
              _media.playerState == 'PAUSED'
                  ? GPlaybackState.PAUSED
                  : GPlaybackState.PLAYING,
            ),
          );
        }
      case (_receiverNamespace, 'RECEIVER_STATUS'):
        final applications = event.data['status']?['applications'];
        final ours =
            applications is List &&
            applications.any(
              (app) => app is Map && app['sessionId'] == session.sessionId,
            );
        // Something else took the screen, or the device stopped us.
        if (!ours) unawaited(_ended(session));
      case (connectionNamespace, 'CLOSE')
          when event.source == session.transportId:
        unawaited(_ended(session));
    }
  }

  void _onMediaStatus(_Session session, Map<String, dynamic> data) {
    final statuses = data['status'];
    final media = _media;
    if (statuses is! List || _loaded == null || _session != session) return;
    if (statuses.isEmpty) {
      // Nothing loaded any more: stopped from the device or another sender.
      if (media.id != null) {
        media
          ..currentTime = media.now
          ..playerState = 'IDLE';
        unawaited(_finish(ended: false));
      }
    }
    for (final status in statuses.whereType<Map<String, dynamic>>()) {
      final id = status['mediaSessionId'] as int?;
      // A status for the title this replaced, which only says it was interrupted.
      if (media.id != null && id != media.id) continue;
      final state = status['playerState'] as String? ?? 'IDLE';
      media.id = id;
      if (state == 'IDLE') {
        // Its time is where the device reset to, not where the viewer got to.
        media
          ..currentTime = media.now
          ..playerState = state;
        unawaited(_finish(ended: status['idleReason'] == 'FINISHED'));
        continue;
      }
      final currentTime = (status['currentTime'] as num?)?.toDouble();
      media
        ..playerState = state
        ..currentTime =
            currentTime != null && (currentTime > 0 || !media.awaitingPosition)
            ? currentTime
            : media.currentTime
        ..heardAt = DateTime.now();
      if (currentTime != null && currentTime > 0) {
        media.awaitingPosition = false;
      }
      final duration = (status['media']?['duration'] as num?)?.toDouble();
      if (duration != null && duration > 0) media.duration = duration;
    }
    _publish();
  }

  void _onHealth(Map<String, dynamic> health) {
    _health = health;
    switch (health['kind']) {
      case 'loaded':
        _auto.settle();
      case 'stalled' when health['seeking'] != true:
        _auto.stalled();
      case 'resumed':
        _auto.resumed();
    }
  }

  /// Auto moves between the original and the ladder; along the ladder, the receiver adapts.
  void _steerAuto() {
    final loaded = _loaded;
    if (loaded == null ||
        _switching ||
        loaded.quality.kind != QualityKind.auto ||
        _session == null ||
        _media.playerState == 'PAUSED') {
      return;
    }
    if (!loaded.adaptive && !loaded.leftOriginal && _auto.struggling()) {
      _auto.leftOriginal();
      unawaited(
        _reload(
          leftOriginal: true,
          notice: 'Lowered the quality to keep up with the connection',
        ),
      );
    } else if (loaded.adaptive &&
        loaded.leftOriginal &&
        _auto.recovered(
          (_health?['estimatedBandwidthBps'] as num?)?.toInt(),
          loaded.sourceBitrateBps,
        )) {
      unawaited(
        _reload(leftOriginal: false, notice: 'Back to the original quality'),
      );
    }
  }

  /// Load what's playing again from where it is, at another quality.
  Future<void> _reload({
    QualityChoice? quality,
    bool leftOriginal = false,
    String? notice,
  }) async {
    final loaded = _loaded;
    final session = _session;
    if (loaded == null || session == null) return;
    _switching = true;
    try {
      final done = await _load(
        session.device,
        loaded.work,
        editionId: loaded.editionId,
        mediaFileId: loaded.mediaFileId,
        positionMs: _positionMs(),
        quality: quality ?? loaded.quality,
        leftOriginal: leftOriginal,
      );
      if (done && notice != null) {
        _toasts.show(notice);
        _session?.channel.send(_noflexNamespace, {
          'type': 'notice',
          'text': notice,
        }, destination: session.transportId);
      }
    } finally {
      _switching = false;
    }
  }

  /// Play what's on the device at [value] from now on, as this device does.
  Future<void> setQuality(QualityChoice value) async {
    await _playback.rememberQuality(value);
    final loaded = _loaded;
    if (loaded != null && value != loaded.quality) {
      await _reload(quality: value);
    }
  }

  /// Show or hide the receiver's stream info panel.
  void showStats(bool show) {
    final session = _session;
    statsOnTv.value = show;
    session?.channel.send(_noflexNamespace, {
      'type': 'stats',
      'show': show,
    }, destination: session.transportId);
  }

  /// What the session's device decodes: NoFlex's receiver probes itself;
  /// Google's is looked up by model.
  Future<GClientCapabilities> _profile(_Session session) {
    final device = session.device;
    GClientCapabilities fallback() => castCapabilities(device.model);
    if (!ownReceiver) return Future.value(fallback());
    return _profiles[device.id] ??= session.channel
        .request(
          _noflexNamespace,
          {'type': 'capabilities'},
          destination: session.transportId,
          idKey: 'id',
        )
        .then((reply) {
          final capabilities = reply.data['capabilities'];
          if (capabilities is! Map<String, dynamic>) {
            throw StateError('${reply.data['error'] ?? 'no capabilities'}');
          }
          return GClientCapabilities.fromJson(capabilities)!;
        })
        .catchError((Object error) {
          debugPrint('Cast device probe failed; going by its model: $error');
          _profiles.remove(device.id);
          return fallback();
        });
  }

  /// Source time now, in milliseconds: where the device last said while it's
  /// out of reach, rather than a guess.
  int _positionMs([_Loaded? title]) {
    final seconds = _session == null ? _media.currentTime : _media.now;
    return math.max(
      0,
      ((title ?? _loaded)?.offsetMs ?? 0) + (seconds * 1000).round(),
    );
  }

  /// Tell the shared list what the Cast devices are doing.
  void _publish() {
    final session = _session;
    final loaded = _loaded;
    final media = _media;
    final live =
        loaded != null &&
        session != null &&
        media.id != null &&
        media.playerState != 'IDLE';
    final away = loaded != null && session == null ? _away : null;
    if (live) {
      final now = media.playerState == 'PAUSED'
          ? GPlaybackState.PAUSED
          : GPlaybackState.PLAYING;
      if (now != _last) {
        _last = now;
        unawaited(_report(now));
      }
    }
    final preparing = _preparing;
    final durationMs =
        loaded?.durationMs ??
        (media.duration == null ? null : (media.duration! * 1000).round());
    final RemotePlayback? playback = live
        ? RemotePlayback(
            state: switch (media.playerState) {
              'PLAYING' => GDevicePlayState.PLAYING,
              'PAUSED' => GDevicePlayState.PAUSED,
              _ => GDevicePlayState.BUFFERING,
            },
            positionMs: loaded.offsetMs + (media.currentTime * 1000).round(),
            durationMs: durationMs,
            work: loaded.work,
          )
        // Where it was, waiting, while the device is found again.
        : away != null
        ? RemotePlayback(
            state: GDevicePlayState.BUFFERING,
            positionMs: _positionMs(),
            durationMs: durationMs,
            work: loaded!.work,
          )
        : preparing != null
        ? RemotePlayback(
            state: GDevicePlayState.BUFFERING,
            positionMs: preparing.positionMs,
            work: preparing.work,
          )
        : null;
    final holder = session?.device ?? away;
    RemoteDevice remote(CastReceiverInfo info) {
      final ours = holder?.id == info.id;
      return RemoteDevice(
        id: _id(info),
        name: info.name,
        // The model says what it is, as a TV with Cast built in reports its own.
        client: [
          if (info.model.isNotEmpty) info.model,
          'Chromecast',
        ].join(' · '),
        heardAt: ours && live ? media.heardAt : DateTime.now(),
        playback: ours ? playback : null,
        via: RemoteVia.cast,
      );
    }

    final found = _discovery.devices.peek();
    _connect.external.value = [
      for (final info in found) remote(info),
      // The device playing stays listed though its announcement lapses.
      if (holder != null && !found.any((info) => info.id == holder.id))
        remote(holder),
    ];
  }

  Future<void> _report(
    GPlaybackState state, [
    _Loaded? title,
    int? capturedPositionMs,
  ]) async {
    final loaded = title ?? _loaded;
    if (loaded == null) return;
    if (state != GPlaybackState.STOPPED &&
        _progressReportedAt != 0 &&
        DateTime.now().millisecondsSinceEpoch - _progressReportedAt < 30000) {
      return;
    }
    _progressReportedAt = 0;
    final positionMs = capturedPositionMs ?? _positionMs(loaded);
    try {
      final data = await _graphql.run(
        GReportProgressReq(
          (b) => b.vars
            ..id = loaded.work.id
            ..position = positionMs
            ..state = state,
        ),
        server: loaded.work.server,
      );
      if (_loaded != loaded &&
          !(state == GPlaybackState.STOPPED && _loaded == null)) {
        return;
      }
      final answer = data.reportProgress;
      getIt<WatchStateService>().set(
        loaded.work.id,
        WatchData(
          played: answer.played,
          positionMs: answer.positionMs,
          favorite: answer.favorite,
        ),
      );
    } on Exception {
      // The next report carries it.
    }
  }

  /// End what was playing: its last position, its watched mark if it ran out,
  /// and, unless it may still be playing, its server session.
  Future<void> _finish({required bool ended, bool stopServer = true}) async {
    final loaded = _loaded;
    _progressReportedAt = 0;
    if (loaded == null) return;
    // Taken at once, so a second idle status in the meantime finds nothing to end.
    _loaded = null;
    _last = null;
    qualities.value = const [];
    _beat?.cancel();
    _steer?.cancel();
    _poll?.cancel();
    final positionMs = ended
        ? loaded.durationMs ?? _positionMs(loaded)
        : _positionMs(loaded);
    await _report(GPlaybackState.STOPPED, loaded, positionMs);
    if (ended) {
      try {
        await _graphql.run(
          GMarkPlayedReq((b) => b.vars.id = loaded.work.id),
          server: loaded.work.server,
        );
        final watch = getIt<WatchStateService>();
        final previous =
            watch.overrideFor(loaded.work.id) ??
            loaded.work.userData ??
            const WatchData();
        watch.set(loaded.work.id, previous.copyWith(played: true));
      } on Exception {
        // Marked the next time it's watched to the end.
      }
    }
    if (stopServer) {
      await _playback.stopSession(loaded.sessionId, server: loaded.work.server);
    }
    _publish();
  }

  Future<_Playable> _resolve(
    WorkSummary work, {
    String? editionId,
    String? mediaFileId,
    int? positionMs,
  }) async {
    final server = work.server;
    final detail = await _graphql.run(
      GWorkDetailReq(
        (b) => b
          ..fetchPolicy = FetchPolicy.NetworkOnly
          ..vars.id = work.id,
      ),
      server: server,
    );
    final editions = detail.editions.toList();
    if (editions.isEmpty) {
      throw ApiError('“${work.title}” has nothing playable attached yet.');
    }
    final edition = editions.firstWhere(
      (e) => e.id == editionId,
      orElse: () => editions.first,
    );
    final files = (await _graphql.run(
      GMediaFilesReq(
        (b) => b
          ..fetchPolicy = FetchPolicy.CacheFirst
          ..vars.edition = edition.id,
      ),
      server: server,
    )).mediaFiles.toList();
    if (files.isEmpty) {
      throw const ApiError('This edition has no media files on disk.');
    }
    final file = files.firstWhere(
      (f) => f.id == mediaFileId,
      orElse: () => files.first,
    );
    final prefs = getIt<SessionService>().playbackPrefs.peek();
    final tracks = subtitleChoicesOf(file);
    final subtitle = preferredSubtitle(tracks, prefs);
    final userData = detail.work?.userData;
    return (
      file: file,
      editionId: edition.id,
      audio: preferredAudio(versionFromFile(file).probe.audio, prefs),
      subtitle: subtitle,
      // The receiver shows text subtitles; a picture one is burned in.
      burn: subtitle != null && tracks[subtitle].isImage ? subtitle : null,
      resume:
          positionMs ??
          ((userData?.played ?? false) ? 0 : userData?.positionMs ?? 0),
      runtimeMs: edition.runtimeMs ?? work.runtimeMs,
    );
  }

  /// The server encodes a transcode's first segments after the session opens.
  Future<void> _awaitPlaylist(String url) async {
    for (var attempt = 0; attempt < 40; attempt++) {
      try {
        final response = await http
            .get(Uri.parse(url))
            .timeout(const Duration(seconds: 12));
        if (response.statusCode == 200) return;
      } on Exception {
        // The encoder isn't there yet.
      }
      await Future<void>.delayed(const Duration(milliseconds: 500));
    }
    throw const ApiError('The stream didn’t start in time.');
  }

  /// Report, steer and ask after the title on the device from now on.
  void _startTimers() {
    _beat?.cancel();
    _steer?.cancel();
    _poll?.cancel();
    _beat = Timer.periodic(_reportEvery, (_) {
      // Out of reach, the device's last word has been reported already.
      final state = _last;
      if (_session != null &&
          state != null &&
          (state == GPlaybackState.PLAYING || _progressReportedAt != 0)) {
        unawaited(_report(state));
      }
    });
    _steer = Timer.periodic(const Duration(seconds: 1), (_) => _steerAuto());
    _poll = Timer.periodic(_statusEvery, (_) {
      final current = _session;
      final id = _media.id;
      if (current != null && id != null) {
        _command(current, {'type': 'GET_STATUS', 'mediaSessionId': id});
      }
    });
    _publish();
  }

  @override
  Future<bool> play(
    RemoteDevice device,
    WorkSummary work, {
    String? editionId,
    String? mediaFileId,
    int? positionMs,
  }) async {
    final info = _infoFor(device.id);
    if (info == null) return false;
    return _load(
      info,
      work,
      editionId: editionId,
      mediaFileId: mediaFileId,
      positionMs: positionMs,
    );
  }

  /// Start [work] on [info], from the viewer's place unless told otherwise.
  Future<bool> _load(
    CastReceiverInfo info,
    WorkSummary work, {
    String? editionId,
    String? mediaFileId,
    int? positionMs,
    QualityChoice? quality,
    bool leftOriginal = false,
  }) async {
    final server = work.server;
    String absolute(String url) => url.startsWith('/')
        ? (server != null ? '${server.origin}$url' : _tokens.absolute(url))
        : url;
    if (_loopback(server?.origin ?? _tokens.origin.peek() ?? '')) {
      _toasts.show(
        'Connect to this server at its network address, not localhost, to cast: a Cast device couldn’t reach it.',
        kind: ToastKind.error,
      );
      return false;
    }
    final progressUrl = absolute('/graphql');
    SessionInfo? opened;
    try {
      final session = await _connectTo(info);
      final playable = await _resolve(
        work,
        editionId: editionId,
        mediaFileId: mediaFileId,
        positionMs: positionMs,
      );
      final chosen = quality ?? _playback.quality.peek();
      final same = _loaded?.mediaFileId == playable.file.id;
      // Leaving the original asks for less than it; only the session that played it knows its rate.
      final original = same ? _loaded?.sourceBitrateBps : null;
      if (!same) _auto = AutoQuality();
      // The receiver buffers while it loads, which is no sign of struggling.
      _auto.settle();
      final ladder = same ? qualities.peek() : const <StreamQuality>[];
      await _finish(ended: false);
      // From here the bar shows it loading, through opening the session and the encoder's first segments.
      _preparing = (work: work, positionMs: playable.resume);
      _publish();
      final stream = await _playback.startSession(
        playable.file.id,
        server: server,
        burn: playable.burn,
        startMs: playable.resume,
        audio: playable.audio,
        quality: sessionQuality(chosen, original, leftOriginal: leftOriginal),
        device: await _profile(session),
      );
      opened = stream;
      if (stream.route != GPlaybackRouteKind.DIRECT) {
        await _awaitPlaylist(stream.url);
      }

      final decision = stream.decision;
      final offsetMs = stream.transcoding ? stream.startPositionMs : 0;
      final steps = ladder.isNotEmpty ? ladder : stream.qualities;
      final pinned = chosen.kind == QualityKind.pinned
          ? steps
                .where((step) => step.bitrateBps == chosen.bitrateBps)
                .firstOrNull
                ?.label
          : null;
      final poster = pickImage(work.images, ArtKind.poster);
      final backdrop = pickImage(work.images, ArtKind.backdrop);
      final subtitles = stream.subtitles;
      final active = subtitles.indexWhere(
        (track) => track.index == playable.subtitle,
      );
      final fmp4 = decision?.target.container == 'fmp4';
      final reply = await session.channel.request(
        _mediaNamespace,
        {
          'type': 'LOAD',
          'autoplay': true,
          // A linear transcode starts at the resume point; a file or an on-demand playlist seeks to it.
          'currentTime': math.max(0, playable.resume - offsetMs) / 1000,
          if (active >= 0) 'activeTrackIds': [active + 1],
          'media': {
            'contentId': stream.url,
            'contentUrl': stream.url,
            'contentType': stream.route == GPlaybackRouteKind.DIRECT
                ? ((decision?.source.container ?? '').contains('webm')
                      ? 'video/webm'
                      : 'video/mp4')
                : 'application/x-mpegURL',
            'streamType': 'BUFFERED',
            // Google's own receiver takes HLS segments for MPEG-TS unless told; ours sets this too.
            if (stream.route != GPlaybackRouteKind.DIRECT && fmp4) ...{
              'hlsSegmentFormat': 'fmp4',
              'hlsVideoSegmentFormat': 'fmp4',
            },
            'metadata': {
              'metadataType': 0,
              'title': work.title,
              'subtitle': work.subtitle,
              if (poster != null)
                'images': [
                  {'url': absolute(poster)},
                ],
            },
            'tracks': [
              for (var i = 0; i < subtitles.length; i++)
                {
                  'trackId': i + 1,
                  'type': 'TEXT',
                  'trackContentId': subtitles[i].url,
                  'trackContentType': 'text/vtt',
                  'subtype': 'SUBTITLES',
                  'name':
                      subtitles[i].title ??
                      subtitles[i].language ??
                      'Track ${i + 1}',
                  'language': ?subtitles[i].language,
                },
            ],
            'customData': {
              'protocol': _protocol,
              if (stream.progressToken != null)
                'progress': {'url': progressUrl, 'token': stream.progressToken},
              'workId': work.id,
              'offsetMs': offsetMs,
              'durationMs': playable.runtimeMs,
              'backdrop': backdrop == null ? null : absolute(backdrop),
              'stream': decision == null
                  ? null
                  : {
                      'route': stream.route.name,
                      'adaptive': stream.adaptive,
                      'container': decision.target.container,
                      'encodes':
                          decision.plan == GPlaybackPlanKind.TRANSCODE_VIDEO ||
                          decision.plan == GPlaybackPlanKind.TRANSCODE,
                      'mode': _plans[decision.plan.name] ?? decision.plan.name,
                      'reasons': [
                        for (final reason in decision.reasons)
                          reason.description,
                      ],
                      'source': decision.source.toJson(),
                      'target': decision.target.toJson(),
                      'encoder': decision.encoder,
                      'hardware': decision.hardware,
                      'quality':
                          pinned ??
                          frameLabel(
                            decision.target.width,
                            decision.target.height,
                          ),
                    },
              // What a sender needs to take this up should this one close or
              // lose the device (_adopt): the device keeps it with the media.
              'sender': {
                'user': _tokens.activeId.peek(),
                'server': server?.id,
                'sessionId': stream.sessionId,
                'mediaFileId': playable.file.id,
                'editionId': playable.editionId,
                'transcode': stream.transcoding,
                'quality': encodeChoice(chosen),
                'leftOriginal': leftOriginal,
                'qualities': [
                  for (final step in steps)
                    {
                      'label': step.label,
                      'width': step.width,
                      'height': step.height,
                      'bitrateBps': step.bitrateBps,
                    },
                ],
              },
            },
          },
        },
        destination: session.transportId,
        timeout: const Duration(seconds: 60),
      );
      if (reply.type != 'MEDIA_STATUS') {
        throw ApiError(
          'That Cast device couldn’t play this (${reply.data['reason'] ?? reply.type}).',
        );
      }
      _media = _Media()
        ..currentTime = math.max(0, playable.resume - offsetMs) / 1000;
      session.owned = true;
      _loaded = _Loaded(
        work: work,
        mediaFileId: playable.file.id,
        editionId: playable.editionId,
        sessionId: stream.sessionId,
        transcode: stream.transcoding,
        offsetMs: offsetMs,
        durationMs: playable.runtimeMs,
        quality: chosen,
        leftOriginal: leftOriginal,
        adaptive: stream.adaptive,
        sourceBitrateBps: decision?.source.bitrateBps,
      );
      _preparing = null;
      _onMediaStatus(session, reply.data);
      qualities.value = steps;
      choice.value = chosen;
      _startTimers();
      return true;
    } on Exception catch (error) {
      _preparing = null;
      if (_loaded == null && opened != null) {
        await _playback.stopSession(opened.sessionId, server: server);
      }
      _publish();
      _toasts.show(
        error is TimeoutException
            ? 'That Cast device didn’t answer.'
            : error.toString(),
        kind: ToastKind.error,
      );
      return false;
    }
  }

  /// Steer the title on the Cast device.
  @override
  Future<void> control(
    RemoteDevice device,
    GDeviceControl action, {
    int? positionMs,
  }) async {
    final session = _session;
    final loaded = _loaded;
    final media = _media;
    final id = media.id;
    if (session == null || loaded == null || id == null) return;
    void send(String type, [Map<String, Object?> extra = const {}]) =>
        _command(session, {'type': type, 'mediaSessionId': id, ...extra});
    switch (action) {
      case GDeviceControl.STOP:
        send('STOP');
        await _finish(ended: false);
        return;
      case GDeviceControl.SEEK when positionMs != null:
        final local = (positionMs - loaded.offsetMs) / 1000;
        final produced = media.duration ?? 0;
        // Beyond what a transcode has produced, a new session starting there beats waiting for the encoder.
        if (loaded.transcode &&
            (local < 0 || local > produced + _transcodeReach)) {
          await _load(
            session.device,
            loaded.work,
            editionId: loaded.editionId,
            mediaFileId: loaded.mediaFileId,
            positionMs: positionMs,
            quality: loaded.quality,
            leftOriginal: loaded.leftOriginal,
          );
          return;
        }
        _auto.settle();
        media
          ..currentTime = local
          ..awaitingPosition = false
          ..heardAt = DateTime.now();
        send('SEEK', {'currentTime': local});
      case GDeviceControl.PAUSE:
        media
          ..currentTime = media.now
          ..heardAt = DateTime.now()
          ..playerState = 'PAUSED';
        send('PAUSE');
      case GDeviceControl.PLAY:
        media
          ..heardAt = DateTime.now()
          ..playerState = 'PLAYING';
        send('PLAY');
      default:
    }
    _publish();
  }

  void dispose() {
    _stopWatchingSession();
    _stopWatchingDevices();
    _stopFollowingTarget();
    _lifecycle.dispose();
    unawaited(_close());
    unawaited(_discovery.stop());
  }
}
