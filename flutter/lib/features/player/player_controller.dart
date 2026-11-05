// SPDX-License-Identifier: AGPL-3.0-only
//
// One player's state, a port of web/src/components/Player.svelte's logic. All of
// media_kit's streams are mirrored into signals here, once; the screen only
// reads signals.
//
// Everything the viewer sees (timeline, chapters, intro, reports, room sync) is
// in source time. A transcode's time zero is `startPositionMs` into the source,
// so source time is `offset + player position`.

import 'dart:async';
import 'dart:io';

import 'package:ferry/ferry.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart' show IconData, KeyEventResult;
import 'package:http/http.dart' as http;
import 'package:media_kit/media_kit.dart' hide SubtitleTrack;
import 'package:media_kit/media_kit.dart' as mk show SubtitleTrack;
import 'package:media_kit_video/media_kit_video.dart';
import 'package:signals_flutter/signals_flutter.dart';
import 'package:wakelock_plus/wakelock_plus.dart';
import 'package:window_manager/window_manager.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/control_watch_room.req.gql.dart';
import '../../graphql/queries/__generated__/mark_played.req.gql.dart';
import '../../graphql/queries/__generated__/report_progress.req.gql.dart';
import '../../graphql/queries/__generated__/watch_room.req.gql.dart';
import '../../graphql/queries/__generated__/watch_room_events.req.gql.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../services/pip_service.dart';
import '../../services/toast_service.dart';
import '../../services/watch_state.dart';
import '../downloads/downloads_service.dart';
import 'decoder_probe.dart';
import 'decoders.dart';
import 'playback_position.dart';
import 'playback_service.dart';

/// libmpv's GL output can't get a context under the Android emulator's EGL
/// translation, so test devices build with `mediacodec_embed`; see AGENTS.md.
const _androidVo = String.fromEnvironment('NOFLEX_ANDROID_VO');

/// How long a stream may take to produce its first playlist.
const _playlistPatience = Duration(minutes: 1);

const _chromeIdle = Duration(milliseconds: 2600);
const _heartbeat = Duration(seconds: 15);

const _streamLost = 'The stream stopped before the end.';

const _landingPatience = Duration(seconds: 30);

bool get _isDesktop =>
    Platform.isLinux || Platform.isMacOS || Platform.isWindows;

class PlaybackStats {
  const PlaybackStats({
    required this.decoder,
    required this.hardware,
    required this.size,
    required this.fps,
    required this.dropped,
    required this.bufferSeconds,
  });

  final String decoder;
  final bool hardware;
  final String size;
  final String fps;
  final String dropped;
  final double bufferSeconds;
}

class PlayerController {
  PlayerController() {
    final decoderApi = _hwdecFor();
    _player = Player(
      configuration: const PlayerConfiguration(
        title: 'NoFlex',
        bufferSize: 64 * 1024 * 1024,
        libass: true,
        libassAndroidFont: 'assets/fonts/Inter-Variable.ttf',
        libassAndroidFontName: 'Inter',
      ),
    );
    video = VideoController(
      _player,
      configuration: VideoControllerConfiguration(
        vo: Platform.isAndroid && _androidVo.isNotEmpty ? _androidVo : null,
        hwdec: decoderApi,
      ),
    );
    _native = _player.platform as NativePlayer;
    _subscriptions.addAll([
      _player.stream.position.listen(_onPosition),
      _player.stream.duration.listen((_) => _updateDuration()),
      _player.stream.buffer.listen(_onBuffer),
      _player.stream.playing.listen(_onPlaying),
      _player.stream.buffering.listen(
        (value) => waiting.value = value || restarting.value != null,
      ),
      _player.stream.completed.listen((done) {
        // A stream on its way out running dry isn't the title ending.
        if (done && restarting.value == null) unawaited(_onEnded());
      }),
      _player.stream.tracks.listen((tracks) {
        audioTracks.value = tracks.audio
            .where((t) => t.id != 'auto' && t.id != 'no')
            .toList();
        embeddedSubtitles.value = tracks.subtitle
            .where((t) => t.id != 'auto' && t.id != 'no')
            .toList();
        _applyTracks();
      }),
      _player.stream.error.listen(_onError),
    ]);
    _stopWatchingCurrent = effect(() {
      final playing = _service.current.value;
      if (playing == null) return;
      final url = playing.url;
      if (url == _openedUrl) return;
      _openedUrl = url;
      untracked(() => unawaited(_open(playing)));
    });
    showChrome();
  }

  PlaybackService get _service => getIt<PlaybackService>();
  GraphqlService get _graphql => getIt<GraphqlService>();

  late final Player _player;
  late final NativePlayer _native;
  late final VideoController video;
  final _subscriptions = <StreamSubscription<dynamic>>[];
  late final void Function() _stopWatchingCurrent;
  String? _openedUrl;
  Timer? _idle;
  Timer? _heartbeatTimer;
  Timer? _countdownTimer;
  Timer? _restartTimer;
  Timer? _flashTimer;
  Timer? _statsTimer;
  Timer? _hwCheck;
  bool _finalized = false;

  /// Where playback has really got to, in source time.
  int _reached = 0;

  /// The last place asked for, where a jump is expected.
  int _requested = 0;
  DateTime? _landingUntil;

  /// Playing the HLS form of a ready file, where [_open] has the rule on seeks.
  bool _onReadyHls = false;
  bool _disposed = false;
  bool _appliedResume = false;

  final position = signal(0);
  final duration = signal(0);
  final buffered = signal(0);
  final playing = signal(false);
  final waiting = signal(true);
  final error = signal('');
  final chromeVisible = signal(true);
  final scrubbing = signal(false);

  /// Where a scrub is pointing, in source time.
  final scrubTarget = signal(0);
  final nextUp = signal<WorkSummary?>(null);
  final countdown = signal<int?>(null);
  final flash = signal<IconData?>(null);
  final audioTracks = signal<List<AudioTrack>>(const []);
  final embeddedSubtitles = signal<List<mk.SubtitleTrack>>(const []);
  final subtitleIndex = signal<int?>(null);
  final burned = signal<int?>(null);
  final audioIndex = signal<int?>(null);
  final infoOpen = signal(false);
  final stats = signal<PlaybackStats?>(null);
  final fullscreen = signal(false);
  final volume = signal(100.0);
  final muted = signal(false);

  /// Source position (ms) a restarted transcode is coming up at.
  final restarting = signal<int?>(null);

  late final inIntro = computed(() {
    final edition = _service.current.value?.edition;
    final end = edition?.introEndMs;
    if (end == null) return false;
    return position.value >= (edition!.introStartMs ?? 0) &&
        position.value < end - 1500;
  });

  late final inCredits = computed(() {
    final start = _service.current.value?.edition?.creditsStartMs;
    return start != null && position.value >= start;
  });

  late final isGuest = computed(() {
    final room = _service.current.value?.room;
    return room != null && !room.isHost;
  });

  ActivePlayback? get _current => _service.current.value;

  String? _hwdecFor() {
    if (Platform.isAndroid && _androidVo == 'mediacodec_embed') {
      return 'mediacodec';
    }
    // Everywhere else auto settles on the best the platform offers and falls
    // back to software by itself, dav1d for AV1.
    return _isDesktop ? 'auto' : 'auto-safe';
  }

  /// Hold until the encoder has written a playlist. libmpv gives up on the first
  /// failed fetch, with a timeout shorter than the server will hold the request
  /// for, and a live channel can't have one until its first segment has aired.
  /// Whatever's still wrong at the deadline is left for libmpv to report.
  Future<void> _awaitPlaylist(String master) async {
    final Uri media;
    try {
      final uri = Uri.parse(master);
      final entry = (await http.get(uri)).body
          .split('\n')
          .map((line) => line.trim())
          .where((line) => line.isNotEmpty && !line.startsWith('#'))
          .firstOrNull;
      if (entry == null) return;
      media = uri.resolve(entry);
    } on Exception {
      return;
    }
    final deadline = DateTime.now().add(_playlistPatience);
    while (!_disposed &&
        _openedUrl == master &&
        DateTime.now().isBefore(deadline)) {
      try {
        final response = await http
            .get(media)
            .timeout(const Duration(seconds: 20));
        if (response.statusCode == 200) return;
      } on Exception {
        // Not there yet, or the request was held too long. Both clear up.
      }
      await Future<void>.delayed(const Duration(milliseconds: 500));
    }
  }

  Future<void> _open(ActivePlayback next) async {
    batch(() {
      error.value = '';
      waiting.value = true;
      _finalized = false;
      _appliedResume = false;
      _reached = next.resumeMs;
      _requested = next.resumeMs;
      _landingUntil = DateTime.now().add(_landingPatience);
      position.value = next.resumeMs;
      subtitleIndex.value = next.subtitleIndex;
      burned.value = next.burnedSubtitle;
      audioIndex.value = next.audioStream;
    });
    if (next.transcoding || next.live) {
      // A transcode's playlist grows as it encodes, which ffmpeg would take for
      // a live stream and join at the edge; its time zero is where to start.
      await _native.setProperty(
        'demuxer-lavf-o',
        next.live ? '' : 'live_start_index=0',
      );
      // Live can't seek, and has to say so: libmpv seeks to attach its video
      // output once playing, and a stream claiming it can never recovers.
      await _native.setProperty('force-seekable', next.live ? 'no' : 'yes');
    } else {
      await _native.setProperty('demuxer-lavf-o', '');
      await _native.setProperty('force-seekable', 'no');
    }
    // SurfaceTexture's conversion knows nothing of PQ or HLG: HDR has to come
    // back to libmpv as frames for its tone-mapping.
    if (Platform.isAndroid && _androidVo.isEmpty) {
      final hdr = next.version?.probe.video?.isHdr ?? false;
      await _native.setProperty('hwdec', hdr ? 'mediacodec-copy' : 'auto-safe');
    }
    if (next.transcoding || next.live) {
      // The server has dropped whatever's playing by now, and an encoder can
      // take longer to start than libmpv has left in its buffer. Hold the last
      // frame meanwhile, with the timeline on where this stream will start.
      batch(() {
        restarting.value ??= next.resumeMs;
        buffered.value = 0;
      });
      if (_player.state.playing) await _player.pause();
      await _awaitPlaylist(next.url);
      if (_disposed || _openedUrl != next.url) return;
    }
    // A transcode already starts at the resume point; direct play seeks to it.
    final local = next.resumeMs - next.offsetMs;
    // Android's libmpv can't seek in the HLS form of a ready file: it reads on
    // to the end without ever landing. The file itself seeks, but having no index
    // it opens by walking every fragment, which over a real connection takes far
    // longer than HLS does. So it's only worth it for what HLS can't do.
    final file = Platform.isAndroid ? next.session?.fileUrl : null;
    final openFile = file != null && local > 1000;
    _onReadyHls = file != null && !openFile;
    await _player.open(
      Media(
        openFile ? file : next.url,
        start: !next.live && local > 1000
            ? Duration(milliseconds: local)
            : null,
      ),
    );
    // A restarted transcode's time zero is its target, so source time is right
    // again from here.
    if (restarting.value != null) restarting.value = null;
    await WakelockPlus.enable();
    _updateDuration();
    if (next.work != null && !next.live) {
      unawaited(
        _service.nextSibling(next.work!).then((item) {
          if (!_disposed) nextUp.value = item;
        }),
      );
    }
    if (next.room != null && !next.room!.isHost) _followRoom(next.room!);
    _hwCheck?.cancel();
    _hwCheck = Timer(const Duration(seconds: 4), _checkDecoder);
  }

  /// The measurement said hardware and libmpv fell back: believe libmpv, so the
  /// next choice between versions knows better.
  Future<void> _checkDecoder() async {
    final video = _current?.version?.probe.video;
    final probe = getIt<DecoderProbeService>();
    final profile = probe.profile.value;
    if (video == null || profile == null || _disposed) return;
    if (profile.tierOf(video) != DecodeTier.hardware) return;
    final used = await _native.getProperty('hwdec-current');
    if (used == 'no') {
      debugPrint(
        'hardware decode fell back to software for ${video.codec} ${video.height}p',
      );
      unawaited(probe.noteSoftwareFallback(profile.withoutHardware(video)));
    }
  }

  void _updateDuration() {
    final current = _current;
    if (current == null) return;
    // A transcode's growing duration would make the timeline creep; the
    // runtime is the real length.
    final own = _player.state.duration.inMilliseconds;
    if (!current.transcoding) {
      // Reopening reads as no length at all until the new stream has one, and
      // the timeline shouldn't collapse under whoever just seeked on it.
      if (own > 0 || (restarting.value == null && _landingUntil == null)) {
        duration.value = own;
      }
      return;
    }
    // This file's own length first: versions of an edition can differ.
    final known =
        current.version?.probe.durationMs ??
        current.edition?.runtimeMs ??
        current.work?.runtimeMs ??
        0;
    duration.value = known > 0 ? known : current.offsetMs + own;
  }

  int get _sourceTime => restarting.value ?? _reached;

  /// A resume reads as the start of the file until it lands, and leaving then
  /// would lose the viewer's place. It gets a while, in case it never does.
  bool _isStillLanding(int at) {
    final until = _landingUntil;
    if (until == null) return false;
    if (isFarFrom(at, _requested) && DateTime.now().isBefore(until)) {
      return true;
    }
    _landingUntil = null;
    return false;
  }

  void _onPosition(Duration value) {
    if (restarting.value != null || scrubbing.value) return;
    final at = (_current?.offsetMs ?? 0) + value.inMilliseconds;
    final falseEnd = isFalseEnd(
      streamMs: value.inMilliseconds,
      streamLengthMs: _player.state.duration.inMilliseconds,
      atMs: at,
      reachedMs: _reached,
      requestedMs: _requested,
    );
    if (falseEnd || _isStillLanding(at)) return;
    _reached = at;
    position.value = at;
    if (inCredits.value &&
        nextUp.value != null &&
        countdown.value == null &&
        _countdownTimer == null &&
        _player.state.playing) {
      _startCountdown();
    }
  }

  void _onBuffer(Duration value) {
    if (restarting.value != null) return;
    buffered.value = (_current?.offsetMs ?? 0) + value.inMilliseconds;
  }

  void _onPlaying(bool isPlaying) {
    playing.value = isPlaying;
    getIt<PipService>().setPlaying(
      isPlaying,
      width: _player.state.width,
      height: _player.state.height,
    );
    _heartbeatTimer?.cancel();
    if (isPlaying) {
      unawaited(_report(GPlaybackState.PLAYING));
      _heartbeatTimer = Timer.periodic(
        _heartbeat,
        (_) => unawaited(_report(GPlaybackState.PLAYING)),
      );
      _roomSend(GWatchRoomAction.PLAY);
      showChrome();
    } else if (!_finalized && restarting.value == null) {
      unawaited(_report(GPlaybackState.PAUSED));
      _roomSend(GWatchRoomAction.PAUSE);
      showChrome();
    }
  }

  void _onError(String message) {
    debugPrint('player: $message');
    // libmpv reports recoverable things here too, like a hardware decoder it
    // then falls back from. It also still calls itself playing when a stream
    // has failed to open, so that has to be picked out by name.
    final state = _player.state;
    if (state.duration > Duration.zero || state.position > Duration.zero) {
      return;
    }
    if (!state.playing || message.startsWith('Failed to open')) {
      error.value = 'This stream couldn’t be played.';
      waiting.value = false;
    }
  }

  Future<void> _report(GPlaybackState state) async {
    final current = _current;
    final work = current?.work;
    if (work == null || current!.live || restarting.value != null) return;
    final positionMs = _sourceTime < 0 ? 0 : _sourceTime;
    final watch = getIt<WatchStateService>();
    final previous =
        watch.overrideFor(work.id) ?? work.userData ?? const WatchData();
    watch.set(work.id, previous.copyWith(positionMs: positionMs));
    final downloads = getIt<DownloadsService>();
    if (current.offline != null) {
      downloads.recordProgress(work.id, positionMs: positionMs);
    }
    try {
      final data = await _graphql.run(
        GReportProgressReq(
          (b) => b.vars
            ..id = work.id
            ..position = positionMs
            ..state = state,
        ),
        server: work.server,
      );
      final answer = data.reportProgress;
      watch.set(
        work.id,
        WatchData(
          played: answer.played,
          positionMs: answer.positionMs,
          favorite: answer.favorite,
        ),
      );
    } on Exception {
      // Carried to the server once it's back; see DownloadsService.
      downloads.queueProgress(work.id, positionMs: positionMs);
    }
  }

  Future<void> _finish() async {
    if (_finalized) return;
    _finalized = true;
    _heartbeatTimer?.cancel();
    await _report(GPlaybackState.STOPPED);
    final current = _current;
    await _service.stopSession(
      current?.session?.sessionId,
      server: current?.server,
    );
  }

  Future<void> close() async {
    if (_closing) return;
    _closing = true;
    await _leaveFullscreen();
    await _finish();
    _service.closed();
    getIt<NavService>().close<void>();
  }

  bool _closing = false;

  Future<void> _onEnded() async {
    final current = _current;
    if (current == null || _finalized) return;
    // libmpv reports a stream it lost the same way, so a server or network going
    // away mid-film would otherwise mark it watched and close the player.
    if (!current.live &&
        endedEarly(lengthMs: duration.value, reachedMs: _sourceTime)) {
      batch(() {
        error.value = _streamLost;
        waiting.value = false;
      });
      return;
    }
    if (current.nextParts.isNotEmpty) {
      await _finish();
      _openedUrl = null;
      if (await _service.playNextPart()) return;
    }
    final work = current.work;
    if (work != null && !current.live) {
      // The server infers completion from progress, but only with a known runtime.
      final watch = getIt<WatchStateService>();
      final previous =
          watch.overrideFor(work.id) ?? work.userData ?? const WatchData();
      watch.set(work.id, previous.copyWith(played: true));
      getIt<DownloadsService>().recordProgress(
        work.id,
        positionMs: 0,
        played: true,
      );
      try {
        await _graphql.run(
          GMarkPlayedReq((b) => b.vars.id = work.id),
          server: work.server,
        );
      } on Exception {
        getIt<DownloadsService>().queueProgress(
          work.id,
          positionMs: 0,
          played: true,
        );
      }
    }
    if (nextUp.value != null) {
      await playNext();
    } else {
      await close();
    }
  }

  Future<void> playNext() async {
    final target = nextUp.value;
    cancelCountdown();
    if (target == null) return close();
    await _finish();
    nextUp.value = null;
    if (!await _service.playNextWork(target)) await close();
  }

  void _startCountdown() {
    countdown.value = 10;
    _countdownTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      final left = (countdown.value ?? 1) - 1;
      countdown.value = left;
      if (left <= 0) unawaited(playNext());
    });
  }

  void cancelCountdown() {
    _countdownTimer?.cancel();
    _countdownTimer = null;
    countdown.value = null;
  }

  // ---- controls ---------------------------------------------------------

  void toggle() {
    if (isGuest.value || restarting.value != null) return;
    unawaited(_player.playOrPause());
    showChrome();
  }

  void seekTo(int ms) {
    final current = _current;
    if (current == null || isGuest.value || current.live) return;
    _seekMedia(ms);
    showChrome();
  }

  void skip(int deltaMs) => seekTo(_sourceTime + deltaMs);

  /// Within what's buffered a seek is native. Before a transcode's offset or
  /// past its buffer, a new session starting at the target beats waiting for
  /// ffmpeg to get there.
  void _seekMedia(int ms) {
    final current = _current!;
    final limit = duration.value > 0 ? duration.value : ms;
    final target = ms.clamp(0, limit);
    final from = _reached;
    position.value = target;
    _requested = target;
    _reached = target;
    _landingUntil = null;
    final local = target - current.offsetMs;
    final buffered = local >= 0 && local <= _player.state.buffer.inMilliseconds;
    final reachable = current.transcoding
        ? buffered
        // Ahead into what's already here is the only seek its HLS form can do.
        : !_onReadyHls || (buffered && target >= from);
    if (reachable) {
      _restartTimer?.cancel();
      if (restarting.value == null) {
        unawaited(_player.seek(Duration(milliseconds: local)));
        _roomSend(
          _player.state.playing
              ? GWatchRoomAction.SEEK
              : GWatchRoomAction.PAUSE,
        );
      } else {
        _restartAt(target);
      }
      return;
    }
    _restartAt(target);
  }

  // Debounced so a run of skips or a drag opens one session, not several.
  void _restartAt(int target) {
    batch(() {
      restarting.value = target;
      waiting.value = true;
    });
    _restartTimer?.cancel();
    _restartTimer = Timer(const Duration(milliseconds: 350), () async {
      _heartbeatTimer?.cancel();
      if (_onReadyHls) {
        // The session's fine; this file just has to be opened the other way.
        await _player.pause();
        _openedUrl = null;
        _service.current.value = _current?.copyWith(resumeMs: target);
        return;
      }
      final ok = await _service.restart(positionMs: target);
      if (!ok) {
        batch(() {
          restarting.value = null;
          waiting.value = false;
          // Nothing's left playing to report this when it runs out.
          if (_player.state.completed) error.value = _streamLost;
        });
      }
    });
  }

  /// A new session can pick up where a lost one stopped. A download has none to
  /// start, and a channel has nowhere to pick up from.
  bool get canRetry {
    final current = _current;
    return current != null &&
        !current.live &&
        current.session != null &&
        current.version != null;
  }

  void retry() {
    final at = _sourceTime;
    error.value = '';
    // A new session's address can match the old one, and it still has to open.
    _openedUrl = null;
    _onReadyHls = false;
    _restartAt(at);
  }

  void startScrub(int ms) {
    if (isGuest.value) return;
    scrubTarget.value = ms;
    scrubbing.value = true;
    showChrome();
  }

  void moveScrub(int ms) {
    scrubTarget.value = ms;
    position.value = ms;
  }

  void endScrub() {
    if (!scrubbing.value) return;
    scrubbing.value = false;
    seekTo(scrubTarget.value);
  }

  void showChrome() {
    chromeVisible.value = true;
    _idle?.cancel();
    _idle = Timer(_chromeIdle, () {
      if (_player.state.playing && !scrubbing.value && !infoOpen.value) {
        chromeVisible.value = false;
      }
    });
  }

  void toggleChrome() {
    if (chromeVisible.value) {
      _idle?.cancel();
      chromeVisible.value = false;
    } else {
      showChrome();
    }
  }

  void pulse(IconData icon) {
    flash.value = icon;
    _flashTimer?.cancel();
    _flashTimer = Timer(
      const Duration(milliseconds: 500),
      () => flash.value = null,
    );
  }

  Future<void> setVolume(double value) async {
    volume.value = value.clamp(0, 100).toDouble();
    muted.value = false;
    await _player.setVolume(volume.value);
  }

  Future<void> toggleMute() async {
    muted.value = !muted.value;
    await _player.setVolume(muted.value ? 0 : volume.value);
  }

  Future<void> toggleFullscreen() async {
    fullscreen.value = !fullscreen.value;
    if (_isDesktop) {
      await windowManager.setFullScreen(fullscreen.value);
    } else {
      await SystemChrome.setEnabledSystemUIMode(
        fullscreen.value
            ? SystemUiMode.immersiveSticky
            : SystemUiMode.edgeToEdge,
      );
    }
  }

  Future<void> enterMobileFullscreen() async {
    if (_isDesktop) return;
    fullscreen.value = true;
    await SystemChrome.setEnabledSystemUIMode(SystemUiMode.immersiveSticky);
  }

  Future<void> _leaveFullscreen() async {
    if (_isDesktop) {
      if (fullscreen.value) await windowManager.setFullScreen(false);
    } else {
      await SystemChrome.setEnabledSystemUIMode(SystemUiMode.edgeToEdge);
      await SystemChrome.setPreferredOrientations(const []);
    }
    fullscreen.value = false;
  }

  // ---- tracks -----------------------------------------------------------

  void _applyTracks() {
    if (_appliedResume) return;
    final current = _current;
    if (current == null ||
        audioTracks.value.isEmpty && embeddedSubtitles.value.isEmpty) {
      return;
    }
    _appliedResume = true;
    if (!current.transcoding) {
      final audio = current.audioStream;
      if (audio != null) unawaited(_selectEmbeddedAudio(audio));
    }
    unawaited(_applySubtitle(subtitleIndex.value));
  }

  Future<void> _selectEmbeddedAudio(int index) async {
    for (final track in audioTracks.value) {
      // libmpv numbers each type's tracks from 1, in container order.
      if (track.id == '${index + 1}') {
        await _player.setAudioTrack(track);
        return;
      }
    }
  }

  Future<void> _applySubtitle(int? index) async {
    final current = _current;
    if (current == null) return;
    if (index == null || burned.value == index) {
      await _player.setSubtitleTrack(mk.SubtitleTrack.no());
      return;
    }
    if (current.offline != null) {
      final subtitle = current.offline!.subtitles
          .where((s) => s.index == index)
          .firstOrNull;
      final path = subtitle == null
          ? null
          : await getIt<DownloadsService>().localSubtitlePath(
              current.offline!,
              subtitle,
            );
      if (path != null) {
        await _player.setSubtitleTrack(
          mk.SubtitleTrack.uri(
            path,
            title: subtitle!.title,
            language: subtitle.language,
          ),
        );
      }
      return;
    }
    final choice = current.subtitleTracks
        .where((t) => t.index == index)
        .firstOrNull;
    // On direct play the container's own tracks are right there, styling and
    // bitmaps intact.
    if (!current.transcoding && choice?.streamIndex != null) {
      for (final track in embeddedSubtitles.value) {
        if (track.id == '${choice!.streamIndex! + 1}') {
          await _player.setSubtitleTrack(track);
          return;
        }
      }
    }
    final delivered = current.session?.subtitles
        .where((s) => s.index == index)
        .firstOrNull;
    if (delivered != null) {
      await _player.setSubtitleTrack(
        mk.SubtitleTrack.uri(
          delivered.url,
          title: delivered.title,
          language: delivered.language,
        ),
      );
    }
  }

  /// A subtitle to show. Only a transcode ever needs the server's help, and only
  /// for a track it can't deliver as text.
  Future<void> chooseSubtitle(int? index) async {
    subtitleIndex.value = index;
    final current = _current;
    if (current == null) return;
    final choice = index == null
        ? null
        : current.subtitleTracks.where((t) => t.index == index).firstOrNull;
    final needsBurn =
        current.transcoding &&
        choice != null &&
        !(current.session?.subtitles.any((s) => s.index == index) ?? false);
    if (needsBurn) {
      burned.value = index;
      await _replaceStream(burn: index);
      return;
    }
    if (burned.value != null) {
      burned.value = null;
      await _replaceStream(clearBurn: true);
    }
    await _applySubtitle(index);
  }

  Future<void> chooseAudio(int index) async {
    audioIndex.value = index;
    final current = _current;
    if (current == null) return;
    if (current.transcoding) {
      await _replaceStream(audio: index);
    } else {
      await _selectEmbeddedAudio(index);
    }
  }

  /// The same place in a stream the server makes differently.
  Future<void> _replaceStream({
    int? burn,
    bool clearBurn = false,
    int? audio,
  }) async {
    final at = _sourceTime;
    restarting.value = at;
    _openedUrl = null;
    final ok = await _service.restart(
      positionMs: at,
      burn: burn,
      clearBurn: clearBurn,
      audio: audio,
    );
    if (!ok) restarting.value = null;
  }

  // ---- watch rooms ------------------------------------------------------

  void _roomSend(GWatchRoomAction action) {
    final room = _current?.room;
    if (room == null || !room.isHost || _suppressRoomEcho) return;
    final at = _sourceTime;
    unawaited(
      _graphql
          .run(
            GControlWatchRoomReq(
              (b) => b.vars
                ..id = room.id
                ..action = action
                ..position = at,
            ),
          )
          .then((_) {}, onError: (Object _) {}),
    );
  }

  bool _suppressRoomEcho = false;
  StreamSubscription<dynamic>? _roomEvents;
  Timer? _roomResync;
  int _roomSequence = -1;

  void _followRoom(RoomRef room) {
    unawaited(_roomEvents?.cancel());
    _roomEvents = _graphql.client
        .request(GWatchRoomEventsReq((b) => b.vars.room = room.id))
        .listen((response) {
          final event = response.data?.watchRoomEvents;
          if (event == null) return;
          final state = event.room;
          if (state.sequence < _roomSequence) return;
          _roomSequence = state.sequence;
          unawaited(
            _applyRoom(state.state.name, state.positionMs, state.updatedAt),
          );
        });
    // Events that fired while the socket was down are lost.
    _roomResync?.cancel();
    _roomResync = Timer.periodic(const Duration(seconds: 30), (_) async {
      try {
        final data = await _graphql.run(
          GWatchRoomReq(
            (b) => b
              ..fetchPolicy = FetchPolicy.NetworkOnly
              ..vars.id = room.id,
          ),
        );
        final state = data.watchRoom;
        if (state != null && state.sequence >= _roomSequence) {
          _roomSequence = state.sequence;
          await _applyRoom(state.state.name, state.positionMs, state.updatedAt);
        }
      } on Exception {
        // Next tick.
      }
    });
  }

  Future<void> _applyRoom(
    String state,
    int roomPositionMs,
    DateTime updatedAt,
  ) async {
    if (state == 'ENDED' || state == 'STOPPED') {
      getIt<ToastService>().show('The host ended the watch room');
      await close();
      return;
    }
    final elapsed = state == 'PLAYING'
        ? DateTime.now().difference(updatedAt).inMilliseconds
        : 0;
    final target = roomPositionMs + (elapsed > 0 ? elapsed : 0);
    _suppressRoomEcho = true;
    if ((target - _sourceTime).abs() > 2000) _seekMedia(target);
    if (state == 'PLAYING' && !_player.state.playing) await _player.play();
    if (state != 'PLAYING' && _player.state.playing) await _player.pause();
    _suppressRoomEcho = false;
  }

  // ---- playback info ----------------------------------------------------

  void toggleInfo() {
    infoOpen.value = !infoOpen.value;
    _statsTimer?.cancel();
    if (infoOpen.value) {
      unawaited(_tickStats());
      _statsTimer = Timer.periodic(
        const Duration(seconds: 1),
        (_) => unawaited(_tickStats()),
      );
    } else {
      showChrome();
    }
  }

  Future<void> _tickStats() async {
    if (_disposed) return;
    final decoder = await _native.getProperty('hwdec-current');
    final fps = await _native.getProperty('estimated-vf-fps');
    final vo = await _native.getProperty('frame-drop-count');
    final dec = await _native.getProperty('decoder-frame-drop-count');
    final width = _player.state.width;
    final height = _player.state.height;
    final ahead = (buffered.value - position.value) / 1000;
    stats.value = PlaybackStats(
      decoder: decoder.isEmpty || decoder == 'no' ? 'Software' : decoder,
      hardware: decoder.isNotEmpty && decoder != 'no',
      size: width != null && height != null ? '$width×$height' : '—',
      fps: double.tryParse(fps)?.toStringAsFixed(2) ?? '—',
      dropped:
          '${vo.isEmpty ? 0 : vo} output, ${dec.isEmpty ? 0 : dec} decoder',
      bufferSeconds: ahead < 0 ? 0 : ahead,
    );
  }

  // ---- keyboard ---------------------------------------------------------

  KeyEventResult onKey(KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    final key = event.logicalKey;
    void Function()? action;
    if (key == LogicalKeyboardKey.space || key == LogicalKeyboardKey.keyK) {
      action = toggle;
    } else if (key == LogicalKeyboardKey.arrowLeft ||
        key == LogicalKeyboardKey.keyJ) {
      action = () => skip(-10000);
    } else if (key == LogicalKeyboardKey.arrowRight ||
        key == LogicalKeyboardKey.keyL) {
      action = () => skip(10000);
    } else if (key == LogicalKeyboardKey.arrowUp) {
      action = () => unawaited(setVolume(volume.value + 5));
    } else if (key == LogicalKeyboardKey.arrowDown) {
      action = () => unawaited(setVolume(volume.value - 5));
    } else if (key == LogicalKeyboardKey.keyM) {
      action = () => unawaited(toggleMute());
    } else if (key == LogicalKeyboardKey.keyF) {
      action = () => unawaited(toggleFullscreen());
    } else if (key == LogicalKeyboardKey.keyN) {
      action = () {
        if (nextUp.value != null) unawaited(playNext());
      };
    } else if (key == LogicalKeyboardKey.keyS) {
      action = () {
        final end = _current?.edition?.introEndMs;
        if (inIntro.value && end != null) seekTo(end);
      };
    } else if (key == LogicalKeyboardKey.keyI) {
      action = toggleInfo;
    } else if (key == LogicalKeyboardKey.escape) {
      action = () {
        if (fullscreen.value && _isDesktop) {
          unawaited(toggleFullscreen());
        } else {
          unawaited(close());
        }
      };
    } else if (key == LogicalKeyboardKey.home) {
      action = () => seekTo(0);
    }
    if (action == null) return KeyEventResult.ignored;
    action();
    showChrome();
    return KeyEventResult.handled;
  }

  void dispose() {
    _disposed = true;
    getIt<PipService>().setPlaying(false);
    _stopWatchingCurrent();
    for (final timer in [
      _idle,
      _heartbeatTimer,
      _countdownTimer,
      _restartTimer,
      _flashTimer,
      _statsTimer,
      _hwCheck,
      _roomResync,
    ]) {
      timer?.cancel();
    }
    for (final subscription in _subscriptions) {
      unawaited(subscription.cancel());
    }
    unawaited(_roomEvents?.cancel());
    unawaited(WakelockPlus.disable());
    unawaited(_player.dispose());
  }
}
