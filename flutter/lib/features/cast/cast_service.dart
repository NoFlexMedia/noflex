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

  // The media session's last status.
  int? mediaSessionId;
  String playerState = 'IDLE';
  double currentTime = 0;
  double? duration;
  DateTime heardAt = DateTime.now();

  /// Seconds into the stream now, advanced since the last status while playing.
  double get now => playerState == 'PLAYING'
      ? currentTime + DateTime.now().difference(heardAt).inMilliseconds / 1000
      : currentTime;
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
      untracked(_publish);
    });
    _stopFollowingTarget = effect(() {
      final id = _connect.targetId.value;
      untracked(() => _choose(id));
    });
    _lifecycle = AppLifecycleListener(
      onStateChange: (state) {
        _foreground = state == AppLifecycleState.resumed;
        _follow();
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

  /// A title being started, which the Cast device hasn't answered for yet.
  ({WorkSummary work, int positionMs})? _preparing;
  var _auto = AutoQuality();

  /// The receiver's latest report on playback.
  Map<String, dynamic>? _health;
  var _switching = false;
  GPlaybackState? _last;
  Timer? _beat;
  Timer? _steer;
  Timer? _status;

  /// What each device decodes, probed once while the app runs.
  final _profiles = <String, Future<GClientCapabilities>>{};

  String _id(CastReceiverInfo device) => 'cast:${device.id}';

  CastReceiverInfo? _infoFor(String id) {
    final session = _session;
    if (session != null && _id(session.device) == id) return session.device;
    for (final device in _discovery.devices.peek()) {
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
    if (!_signedIn) unawaited(_close());
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

  Future<_Session> _connectTo(CastReceiverInfo info) async {
    final current = _session;
    if (current != null && current.device.id == info.id) return current;
    final opening = _opening;
    if (opening != null) {
      final session = await opening;
      if (session.device.id == info.id) return session;
    }
    if (_session != null) await _close();
    return _opening ??= _open(info).whenComplete(() => _opening = null);
  }

  Future<_Session> _open(CastReceiverInfo info) async {
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
      if (app == null) {
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
      final session = _Session(info, channel, app!.transportId, app.sessionId);
      channel.open(session.transportId);
      session.events = channel.events.listen(
        (event) => _onEvent(session, event),
      );
      unawaited(channel.closed.then((_) => _lost(session)));
      _session = session;
      _command(session, {'type': 'GET_STATUS'});
      // Probe while the viewer picks a title, so loading doesn't wait on it.
      unawaited(_profile(session));
      _publish();
      return session;
    } on Object {
      await channel.close();
      rethrow;
    }
  }

  /// The connection went: whatever played there is ours to close out.
  void _lost(_Session session) {
    if (_session != session) return;
    _session = null;
    unawaited(session.events?.cancel());
    _status?.cancel();
    unawaited(_finish(ended: false));
    _publish();
  }

  /// End the session: the app on the device and the connection to it.
  Future<void> _close() async {
    final session = _session;
    if (session == null) return;
    await _finish(ended: false);
    session.channel.send(_receiverNamespace, {
      'type': 'STOP',
      'sessionId': session.sessionId,
      'requestId': 0,
    });
    _lost(session);
    await session.channel.close();
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
      case (_receiverNamespace, 'RECEIVER_STATUS'):
        final applications = event.data['status']?['applications'];
        final ours =
            applications is List &&
            applications.any(
              (app) => app is Map && app['sessionId'] == session.sessionId,
            );
        // Something else took the screen, or the device stopped us.
        if (!ours) unawaited(session.channel.close());
      case (connectionNamespace, 'CLOSE')
          when event.source == session.transportId:
        unawaited(session.channel.close());
    }
  }

  void _onMediaStatus(_Session session, Map<String, dynamic> data) {
    final statuses = data['status'];
    if (statuses is! List) return;
    if (statuses.isEmpty) {
      // Nothing loaded any more: stopped from the device or another sender.
      if (_loaded != null && session.mediaSessionId != null) {
        session.playerState = 'IDLE';
        unawaited(_finish(ended: false));
      }
    }
    for (final status in statuses.whereType<Map<String, dynamic>>()) {
      final id = status['mediaSessionId'] as int?;
      // A status for the title this replaced, which only says it was interrupted.
      if (session.mediaSessionId != null && id != session.mediaSessionId) {
        continue;
      }
      session.mediaSessionId = id;
      session.playerState = status['playerState'] as String? ?? 'IDLE';
      session.currentTime =
          (status['currentTime'] as num?)?.toDouble() ?? session.currentTime;
      session.heardAt = DateTime.now();
      final duration = (status['media']?['duration'] as num?)?.toDouble();
      if (duration != null && duration > 0) session.duration = duration;
      if (session.playerState == 'IDLE' && _loaded != null) {
        unawaited(_finish(ended: status['idleReason'] == 'FINISHED'));
      }
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
        _session?.playerState == 'PAUSED') {
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

  /// Source time now, in milliseconds.
  int _positionMs() => math.max(
    0,
    (_loaded?.offsetMs ?? 0) + ((_session?.now ?? 0) * 1000).round(),
  );

  /// Tell the shared list what the Cast devices are doing.
  void _publish() {
    final session = _session;
    final loaded = _loaded;
    final live =
        loaded != null &&
        session?.mediaSessionId != null &&
        session?.playerState != 'IDLE';
    if (live) {
      final now = session!.playerState == 'PAUSED'
          ? GPlaybackState.PAUSED
          : GPlaybackState.PLAYING;
      if (now != _last) {
        _last = now;
        unawaited(_report(now));
      }
    }
    final preparing = _preparing;
    final RemotePlayback? playback = live
        ? RemotePlayback(
            state: switch (session!.playerState) {
              'PLAYING' => GDevicePlayState.PLAYING,
              'PAUSED' => GDevicePlayState.PAUSED,
              _ => GDevicePlayState.BUFFERING,
            },
            positionMs: loaded.offsetMs + (session.currentTime * 1000).round(),
            durationMs:
                loaded.durationMs ??
                (session.duration == null
                    ? null
                    : (session.duration! * 1000).round()),
            work: loaded.work,
          )
        : preparing != null
        ? RemotePlayback(
            state: GDevicePlayState.BUFFERING,
            positionMs: preparing.positionMs,
            work: preparing.work,
          )
        : null;
    RemoteDevice remote(CastReceiverInfo info) {
      final ours = session?.device.id == info.id;
      return RemoteDevice(
        id: _id(info),
        name: info.name,
        // The model says what it is, as a TV with Cast built in reports its own.
        client: [
          if (info.model.isNotEmpty) info.model,
          'Chromecast',
        ].join(' · '),
        heardAt: ours ? session!.heardAt : DateTime.now(),
        playback: ours ? playback : null,
        via: RemoteVia.cast,
      );
    }

    final found = _discovery.devices.peek();
    _connect.external.value = [
      for (final info in found) remote(info),
      // The device playing stays listed though its announcement lapses.
      if (session != null && !found.any((info) => info.id == session.device.id))
        remote(session.device),
    ];
  }

  Future<void> _report(GPlaybackState state, [_Loaded? title]) async {
    final loaded = title ?? _loaded;
    if (loaded == null) return;
    final positionMs = math.max(
      0,
      loaded.offsetMs + ((_session?.now ?? 0) * 1000).round(),
    );
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
  /// and its server session.
  Future<void> _finish({required bool ended}) async {
    final loaded = _loaded;
    if (loaded == null) return;
    // Taken at once, so a second idle status in the meantime finds nothing to end.
    _loaded = null;
    _last = null;
    qualities.value = const [];
    _beat?.cancel();
    _steer?.cancel();
    _status?.cancel();
    await _report(GPlaybackState.STOPPED, loaded);
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
    await _playback.stopSession(loaded.sessionId, server: loaded.work.server);
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
      session.mediaSessionId = null;
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
      _beat = Timer.periodic(_reportEvery, (_) {
        if (_last == GPlaybackState.PLAYING) {
          unawaited(_report(GPlaybackState.PLAYING));
        }
      });
      _steer = Timer.periodic(const Duration(seconds: 1), (_) => _steerAuto());
      _status = Timer.periodic(_statusEvery, (_) {
        final current = _session;
        if (current != null && current.mediaSessionId != null) {
          _command(current, {
            'type': 'GET_STATUS',
            'mediaSessionId': current.mediaSessionId,
          });
        }
      });
      _publish();
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
    final media = session?.mediaSessionId;
    if (session == null || loaded == null || media == null) return;
    void send(String type, [Map<String, Object?> extra = const {}]) =>
        _command(session, {'type': type, 'mediaSessionId': media, ...extra});
    switch (action) {
      case GDeviceControl.STOP:
        send('STOP');
        await _finish(ended: false);
        return;
      case GDeviceControl.SEEK when positionMs != null:
        final local = (positionMs - loaded.offsetMs) / 1000;
        final produced = session.duration ?? 0;
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
        session
          ..currentTime = local
          ..heardAt = DateTime.now();
        send('SEEK', {'currentTime': local});
      case GDeviceControl.PAUSE:
        session
          ..currentTime = session.now
          ..heardAt = DateTime.now()
          ..playerState = 'PAUSED';
        send('PAUSE');
      case GDeviceControl.PLAY:
        session
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
