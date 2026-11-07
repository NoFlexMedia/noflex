// SPDX-License-Identifier: AGPL-3.0-only
//
// Resolves a work down to a playable file and opens a session, a port of
// web/src/lib/playback.svelte.ts. Where the web takes the first edition and
// file, this ranks them for what the device decodes in hardware.

import 'dart:async';
import 'dart:io';

import 'package:ferry/ferry.dart';
import 'package:flutter/widgets.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/child_page.req.gql.dart';
import '../../graphql/queries/__generated__/media_files.req.gql.dart';
import '../../graphql/queries/__generated__/start_live_channel.req.gql.dart';
import '../../graphql/queries/__generated__/start_playback.data.gql.dart';
import '../../graphql/queries/__generated__/start_playback.req.gql.dart';
import '../../graphql/queries/__generated__/stop_playback.req.gql.dart';
import '../../graphql/queries/__generated__/work_detail.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/edition_full.data.gql.dart';
import '../../graphql/queries/fragments/__generated__/media_file_full.data.gql.dart';
import '../../models/media_probe.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
import '../../utils/tracks.dart';
import '../downloads/download_models.dart';
import '../connect/connect_service.dart';
import '../downloads/downloads_service.dart';
import '../home/home_service.dart';
import 'capabilities.dart';
import 'decoder_probe.dart';
import 'decoders.dart';
import 'player_screen.dart';

typedef PlaybackDecision = GStartPlaybackData_startPlaybackSession_decision;

class DeliveredSubtitle {
  const DeliveredSubtitle({
    required this.index,
    required this.url,
    this.language,
    this.title,
    this.forced = false,
    this.isDefault = false,
  });

  /// Position in the file's `subtitleTracks`.
  final int index;
  final String url;
  final String? language;
  final String? title;
  final bool forced;
  final bool isDefault;
}

class SessionInfo {
  const SessionInfo({
    required this.sessionId,
    required this.route,
    required this.url,
    this.fileUrl,
    this.startPositionMs = 0,
    this.decision,
    this.subtitles = const [],
  });

  final String sessionId;
  final GPlaybackRouteKind route;
  final String url;

  /// The whole file behind [url], when that's a ready version.
  final String? fileUrl;

  /// Source position of the stream's time zero: a transcode's start offset.
  final int startPositionMs;
  final PlaybackDecision? decision;
  final List<DeliveredSubtitle> subtitles;

  bool get transcoding => route == GPlaybackRouteKind.TRANSCODE;
}

class RoomRef {
  const RoomRef({required this.id, required this.isHost});
  final String id;
  final bool isHost;
}

/// What the player is playing. Replaced whole on a restart.
class ActivePlayback {
  const ActivePlayback({
    required this.title,
    required this.subtitle,
    this.work,
    this.server,
    this.edition,
    this.version,
    this.nextParts = const [],
    this.session,
    this.localPath,
    this.resumeMs = 0,
    this.audioStream,
    this.subtitleIndex,
    this.burnedSubtitle,
    this.subtitleTracks = const [],
    this.live = false,
    this.room,
    this.offline,
  });

  final WorkSummary? work;
  final String title;
  final String subtitle;
  final ServerRef? server;
  final GEditionFull? edition;
  final MediaVersion? version;

  /// Later parts of a multi-file cut, best version of each, in order.
  final List<MediaVersion> nextParts;
  final SessionInfo? session;

  /// A downloaded file; no session and nothing to tell the server until later.
  final String? localPath;
  final int resumeMs;

  /// Index among the file's audio streams.
  final int? audioStream;

  /// Index into [subtitleTracks] to show.
  final int? subtitleIndex;

  /// The same index, when it's burned into a transcode's picture.
  final int? burnedSubtitle;
  final List<SubtitleChoice> subtitleTracks;
  final bool live;
  final RoomRef? room;
  final OfflineItem? offline;

  String get url => localPath ?? session!.url;
  bool get transcoding => session?.transcoding ?? false;
  int get offsetMs => transcoding ? session!.startPositionMs : 0;

  ActivePlayback copyWith({
    SessionInfo? session,
    int? resumeMs,
    int? audioStream,
    int? burnedSubtitle,
    bool clearBurned = false,
    int? subtitleIndex,
    bool clearSubtitle = false,
  }) => ActivePlayback(
    title: title,
    subtitle: subtitle,
    work: work,
    server: server,
    edition: edition,
    version: version,
    nextParts: nextParts,
    session: session ?? this.session,
    localPath: localPath,
    resumeMs: resumeMs ?? this.resumeMs,
    audioStream: audioStream ?? this.audioStream,
    burnedSubtitle: clearBurned
        ? null
        : (burnedSubtitle ?? this.burnedSubtitle),
    subtitleIndex: clearSubtitle ? null : (subtitleIndex ?? this.subtitleIndex),
    subtitleTracks: subtitleTracks,
    live: live,
    room: room,
    offline: offline,
  );
}

MediaVersion versionFromFile(GMediaFileFull file) => MediaVersion(
  id: file.id,
  editionId: file.editionId,
  partIndex: file.partIndex,
  sizeBytes: file.sizeBytes,
  path: file.path,
  container: file.container,
  probe: MediaProbe.fromJson(file.probe.value),
);

List<SubtitleChoice> subtitleChoicesOf(GMediaFileFull file) => [
  for (var i = 0; i < file.subtitleTracks.length; i++)
    SubtitleChoice(
      index: i,
      codec: file.subtitleTracks[i].codec,
      language: file.subtitleTracks[i].language,
      title: file.subtitleTracks[i].title,
      isDefault: file.subtitleTracks[i].Gdefault,
      forced: file.subtitleTracks[i].forced,
      streamIndex: file.subtitleTracks[i].streamIndex,
      external: file.subtitleTracks[i].external,
    ),
];

/// Physical pixels of the screen, the most picture a version can usefully have.
/// The screen rather than the window: a small window may yet go fullscreen.
int displayPixels() {
  final views = WidgetsBinding.instance.platformDispatcher.views;
  if (views.isEmpty) return pixels1080p;
  final view = views.first;
  final window = view.physicalSize.width * view.physicalSize.height;
  final screen = view.display.size.width * view.display.size.height;
  final pixels = (screen > window ? screen : window).round();
  return pixels > 0 ? pixels : pixels1080p;
}

const _maxBitrateKey = 'noflex.maxBitrateBps';

class PlaybackService {
  PlaybackService() {
    unawaited(_loadPrefs());
  }

  GraphqlService get _graphql => getIt<GraphqlService>();
  DecoderProbeService get _decoders => getIt<DecoderProbeService>();
  ToastService get _toasts => getIt<ToastService>();

  final current = signal<ActivePlayback?>(null);

  /// The id of the work a session is being opened for.
  final preparing = signal<String?>(null);

  /// A ceiling the viewer set for streaming; null is whatever the device plays.
  final maxBitrateBps = signal<int?>(null);

  Future<void> _loadPrefs() async {
    final prefs = await SharedPreferences.getInstance();
    maxBitrateBps.value = prefs.getInt(_maxBitrateKey);
  }

  Future<void> setMaxBitrate(int? value) async {
    maxBitrateBps.value = value;
    final prefs = await SharedPreferences.getInstance();
    if (value == null) {
      await prefs.remove(_maxBitrateKey);
    } else {
      await prefs.setInt(_maxBitrateKey, value);
    }
  }

  String _absolute(String url, ServerRef? server) => url.startsWith('/')
      ? (server != null
            ? '${server.origin}$url'
            : getIt<TokenStore>().absolute(url))
      : url;

  /// Open a session. A transcode starts at [startMs] so far seeks don't wait on
  /// ffmpeg.
  Future<SessionInfo> startSession(
    String mediaFileId, {
    ServerRef? server,
    int? burn,
    int startMs = 0,
    int? audio,
  }) async {
    final profile = await _decoders.ready();
    final capabilities = buildCapabilities(
      profile,
      maxBitrateBps: maxBitrateBps.value,
    );
    final data = await _graphql.run(
      GStartPlaybackReq(
        (b) => b.vars
          ..media = mediaFileId
          ..capabilities.replace(capabilities)
          ..subtitle = burn
          ..start = startMs < 0 ? 0 : startMs
          ..audio = audio,
      ),
      server: server,
    );
    final session = data.startPlaybackSession;
    return SessionInfo(
      sessionId: session.sessionId,
      route: session.route,
      url: _absolute(session.url, server),
      fileUrl: switch (session.fileUrl) {
        final file? => _absolute(file, server),
        null => null,
      },
      startPositionMs: session.startPositionMs,
      decision: session.decision,
      subtitles: [
        for (final track in session.subtitles)
          DeliveredSubtitle(
            index: track.index,
            url: _absolute(track.url, server),
            language: track.language,
            title: track.title,
            forced: track.forced,
            isDefault: track.Gdefault,
          ),
      ],
    );
  }

  /// Best effort: a dead session is already gone.
  Future<void> stopSession(String? sessionId, {ServerRef? server}) async {
    if (sessionId == null) return;
    try {
      await _graphql.run(
        GStopPlaybackReq((b) => b.vars.session = sessionId),
        server: server,
      );
    } on Exception {
      // Nothing to do about it.
    }
  }

  /// Resolve a work to a file, open a session, and bring up the player.
  /// [editionId] and [mediaFileId] are what the viewer chose; left out, this
  /// picks what the device decodes in hardware.
  Future<bool> playWork(
    WorkSummary work, {
    String? editionId,
    String? mediaFileId,
    int? audioStream,
    int? subtitleIndex,
    bool subtitleChosen = false,
    bool fromStart = false,
    int? positionMs,
    RoomRef? room,
  }) async {
    if (preparing.value != null) return false;
    preparing.value = work.id;
    try {
      // A chosen device plays it, resuming from the viewer's place unless told otherwise.
      if (room == null && getIt<ConnectService>().target.peek() != null) {
        return await getIt<ConnectService>().cast(
          work,
          editionId: editionId,
          mediaFileId: mediaFileId,
          positionMs: fromStart ? 0 : positionMs,
        );
      }
      final downloaded = getIt<DownloadsService>().completedFor(work.id);
      if (downloaded != null && editionId == null && mediaFileId == null) {
        return await _playDownloaded(
          downloaded,
          fromStart: fromStart,
          positionMs: positionMs,
        );
      }
      final next = await _resolve(
        work,
        editionId: editionId,
        mediaFileId: mediaFileId,
        audioStream: audioStream,
        subtitleIndex: subtitleIndex,
        subtitleChosen: subtitleChosen,
        fromStart: fromStart,
        positionMs: positionMs,
        room: room,
      );
      _show(next);
      return true;
    } on SessionExpired {
      return false;
    } on Exception catch (error) {
      _toasts.show(error.toString(), kind: ToastKind.error);
      return false;
    } finally {
      preparing.value = null;
    }
  }

  Future<ActivePlayback> _resolve(
    WorkSummary work, {
    String? editionId,
    String? mediaFileId,
    int? audioStream,
    int? subtitleIndex,
    bool subtitleChosen = false,
    bool fromStart = false,
    int? positionMs,
    RoomRef? room,
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

    // Asked of every edition only when one hasn't been chosen.
    final candidates = editionId != null
        ? editions.where((e) => e.id == editionId).toList()
        : editions;
    final filesByEdition = <String, List<GMediaFileFull>>{};
    await Future.wait([
      for (final edition in candidates)
        _graphql
            .run(
              GMediaFilesReq(
                (b) => b
                  ..fetchPolicy = FetchPolicy.CacheFirst
                  ..vars.edition = edition.id,
              ),
              server: server,
            )
            .then(
              (data) => filesByEdition[edition.id] = data.mediaFiles.toList(),
            ),
    ]);

    final profile = await _decoders.ready();
    final pixels = displayPixels();
    GMediaFileFull? file;
    if (mediaFileId != null) {
      for (final files in filesByEdition.values) {
        for (final candidate in files) {
          if (candidate.id == mediaFileId) file = candidate;
        }
      }
    } else {
      final choice = pickEdition(
        [
          for (final edition in candidates)
            (
              edition.id,
              [
                for (final f
                    in filesByEdition[edition.id] ?? const <GMediaFileFull>[])
                  versionFromFile(f),
              ],
            ),
        ],
        profile,
        displayPixels: pixels,
      );
      final best = choice?.best;
      if (best != null) {
        file = filesByEdition[choice!.editionId]!.firstWhere(
          (f) => f.id == best.id,
        );
      }
    }
    if (file == null) {
      throw const ApiError('This edition has no media files on disk.');
    }
    final chosenFile = file;

    final edition = editions.firstWhere((e) => e.id == chosenFile.editionId);
    final version = versionFromFile(chosenFile);
    final session = getIt<SessionService>();
    final prefs = session.playbackPrefs.value;
    final audio = audioStream ?? preferredAudio(version.probe.audio, prefs);
    final tracks = subtitleChoicesOf(chosenFile);
    final subtitle = subtitleChosen
        ? subtitleIndex
        : preferredSubtitle(tracks, prefs);
    final userData = detail.work?.userData;
    final resume =
        positionMs ??
        (fromStart || (userData?.played ?? false)
            ? 0
            : userData?.positionMs ?? 0);

    // Everything plays through libmpv, which switches in-file tracks and draws
    // image subtitles itself. Only a transcode needs the server to do either.
    var opened = await startSession(
      chosenFile.id,
      server: server,
      startMs: resume,
    );
    final burn =
        opened.transcoding && subtitle != null && tracks[subtitle].isImage
        ? subtitle
        : null;
    final defaultAudio = preferredAudio(
      version.probe.audio,
      const PlaybackPrefs(),
    );
    if (opened.transcoding &&
        (burn != null || (audio != null && audio != defaultAudio))) {
      await stopSession(opened.sessionId, server: server);
      opened = await startSession(
        chosenFile.id,
        server: server,
        burn: burn,
        startMs: resume,
        audio: audio,
      );
    }

    final laterParts = _laterParts(
      filesByEdition[edition.id] ?? const [],
      chosenFile.partIndex,
      profile,
      pixels,
    );
    final title = detail.work?.title ?? work.title;
    return ActivePlayback(
      work: work,
      title: title,
      subtitle: work.subtitle,
      server: server,
      edition: edition,
      version: version,
      nextParts: laterParts,
      session: opened,
      resumeMs: resume,
      audioStream: audio,
      subtitleIndex: subtitle,
      burnedSubtitle: burn,
      subtitleTracks: tracks,
      room: room,
    );
  }

  List<MediaVersion> _laterParts(
    List<GMediaFileFull> files,
    int after,
    DecoderProfile profile,
    int pixels,
  ) {
    final versions = [for (final file in files) versionFromFile(file)];
    final parts =
        versions
            .map((v) => v.partIndex)
            .where((part) => part > after)
            .toSet()
            .toList()
          ..sort();
    return [
      for (final part in parts)
        rankVersions(
          [
            for (final v in versions)
              if (v.partIndex == part) v,
          ],
          profile,
          displayPixels: pixels,
        ).first,
    ];
  }

  Future<bool> _playDownloaded(
    OfflineItem item, {
    bool fromStart = false,
    int? positionMs,
  }) async {
    final downloads = getIt<DownloadsService>();
    final path = await downloads.localPathOf(item);
    if (path == null || !File(path).existsSync()) {
      _toasts.show(
        'That download is missing from this device.',
        kind: ToastKind.error,
      );
      return false;
    }
    final resume =
        positionMs ?? (fromStart || item.played ? 0 : item.positionMs);
    _show(
      ActivePlayback(
        work: item.work,
        title: item.work.title,
        subtitle: item.work.subtitle,
        localPath: path,
        resumeMs: resume,
        offline: item,
        subtitleTracks: [
          for (final subtitle in item.subtitles)
            SubtitleChoice(
              index: subtitle.index,
              codec: 'webvtt',
              language: subtitle.language,
              title: subtitle.title,
              isDefault: subtitle.isDefault,
              forced: subtitle.forced,
              external: true,
            ),
        ],
      ),
    );
    return true;
  }

  /// Play a recording-free live channel.
  Future<void> playLive({
    required String channelId,
    required String name,
  }) async {
    if (preparing.value != null) return;
    preparing.value = channelId;
    try {
      final data = await _graphql.run(
        GStartLiveChannelReq((b) => b.vars.id = channelId),
      );
      final started = data.startLiveTvChannel;
      _show(
        ActivePlayback(
          title: name,
          subtitle: 'Live',
          live: true,
          session: SessionInfo(
            sessionId: started.jobId ?? '',
            route: started.route,
            url: _absolute(started.url, null),
          ),
        ),
      );
    } on SessionExpired {
      return;
    } on Exception catch (error) {
      _toasts.show(error.toString(), kind: ToastKind.error);
    } finally {
      preparing.value = null;
    }
  }

  /// Play a recording-sized stream of one room's file; guests follow the host.
  Future<void> playRoom({
    required RoomRef room,
    required String mediaFileId,
    required int positionMs,
    WorkSummary? work,
  }) async {
    if (work != null) {
      await playWork(
        work,
        mediaFileId: mediaFileId,
        positionMs: positionMs,
        room: room,
      );
      return;
    }
    try {
      final session = await startSession(mediaFileId, startMs: positionMs);
      _show(
        ActivePlayback(
          title: 'Watch room',
          subtitle: '',
          session: session,
          resumeMs: positionMs,
          room: room,
        ),
      );
    } on Exception catch (error) {
      _toasts.show(error.toString(), kind: ToastKind.error);
    }
  }

  void _show(ActivePlayback next) {
    final showing = current.value != null;
    current.value = next;
    if (!showing) {
      unawaited(getIt<NavService>().pushFullWindow(const PlayerScreen()));
    }
  }

  /// Replace the session with one starting at [positionMs]: to burn in a
  /// different subtitle, change a transcode's audio, or seek one far from its
  /// offset. False (after a toast) when no new session could be opened.
  Future<bool> restart({
    required int positionMs,
    int? burn,
    bool clearBurn = false,
    int? audio,
  }) async {
    final playing = current.value;
    final version = playing?.version;
    if (playing == null || version == null) return false;
    try {
      await stopSession(playing.session?.sessionId, server: playing.server);
      final session = await startSession(
        version.id,
        server: playing.server,
        burn: clearBurn ? null : (burn ?? playing.burnedSubtitle),
        startMs: positionMs,
        audio: audio ?? playing.audioStream,
      );
      current.value = playing.copyWith(
        session: session,
        resumeMs: positionMs,
        audioStream: audio,
        burnedSubtitle: burn,
        clearBurned: clearBurn,
      );
      return true;
    } on Exception catch (error) {
      _toasts.show(error.toString(), kind: ToastKind.error);
      return false;
    }
  }

  /// Move on to the next part of a multi-file cut.
  Future<bool> playNextPart() async {
    final playing = current.value;
    if (playing == null || playing.nextParts.isEmpty) return false;
    final next = playing.nextParts.first;
    try {
      await stopSession(playing.session?.sessionId, server: playing.server);
      final session = await startSession(next.id, server: playing.server);
      current.value = ActivePlayback(
        work: playing.work,
        title: playing.title,
        subtitle: 'Part ${next.partIndex + 1}',
        server: playing.server,
        edition: playing.edition,
        version: next,
        nextParts: playing.nextParts.sublist(1),
        session: session,
        room: playing.room,
      );
      return true;
    } on Exception catch (error) {
      _toasts.show(error.toString(), kind: ToastKind.error);
      return false;
    }
  }

  /// Swap the player over to another work.
  Future<bool> playNextWork(WorkSummary next) async {
    preparing.value = next.id;
    try {
      final downloaded = getIt<DownloadsService>().completedFor(next.id);
      if (downloaded != null) {
        return await _playDownloaded(downloaded, fromStart: true);
      }
      current.value = await _resolve(next, fromStart: true);
      return true;
    } on Exception catch (error) {
      _toasts.show(error.toString(), kind: ToastKind.error);
      return false;
    } finally {
      preparing.value = null;
    }
  }

  void closed() {
    current.value = null;
    if (getIt<SessionService>().phase.value == AppPhase.app) {
      getIt<HomeService>().refresh();
      unawaited(getIt<DownloadsService>().reconcile());
    }
  }

  /// Every direct child of [parentId], in index order. Pages through the cursor
  /// because the request-complexity budget caps a page at 60 cards.
  Future<List<WorkSummary>> fetchChildren(
    String libraryId,
    String parentId, {
    ServerRef? server,
  }) async {
    final items = <WorkSummary>[];
    String? cursor;
    do {
      final page = await _graphql.run(
        GChildPageReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NetworkOnly
            ..vars.library = libraryId
            ..vars.parent = parentId
            ..vars.after = cursor,
        ),
        server: server,
      );
      items.addAll([
        for (final node in page.works.nodes)
          WorkSummary.fromCard(node, overview: node.overview, server: server),
      ]);
      cursor = page.works.pageInfo.hasNextPage
          ? page.works.pageInfo.endCursor
          : null;
    } while (cursor != null);
    items.sort(byIndex);
    return items;
  }

  /// The episode or track after [work] within its parent, if any.
  Future<WorkSummary?> nextSibling(WorkSummary work) async {
    final parent = work.parentId;
    if (parent == null || !(work.kind == 'EPISODE' || work.kind == 'TRACK')) {
      return null;
    }
    try {
      final siblings = await fetchChildren(
        work.libraryId,
        parent,
        server: work.server,
      );
      final index = siblings.indexWhere((item) => item.id == work.id);
      return index >= 0 && index + 1 < siblings.length
          ? siblings[index + 1]
          : null;
    } on Exception {
      return null;
    }
  }
}

int byIndex(WorkSummary a, WorkSummary b) {
  final byParent = (a.parentIndex ?? 0).compareTo(b.parentIndex ?? 0);
  if (byParent != 0) return byParent;
  final byNumber = (a.indexNumber ?? 1000000000).compareTo(
    b.indexNumber ?? 1000000000,
  );
  if (byNumber != 0) return byNumber;
  return a.title.compareTo(b.title);
}
