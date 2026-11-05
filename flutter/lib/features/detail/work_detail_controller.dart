// SPDX-License-Identifier: AGPL-3.0-only
//
// A title's page state: the query, which edition and version are chosen, and what
// a show should play next. Editions and versions start on what this device decodes
// in hardware; once the viewer picks, their pick stands.

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/queries/__generated__/mark_played.req.gql.dart';
import '../../graphql/queries/__generated__/mark_unplayed.req.gql.dart';
import '../../graphql/queries/__generated__/media_files.req.gql.dart';
import '../../graphql/queries/__generated__/server_info.req.gql.dart';
import '../../graphql/queries/__generated__/set_favorite.req.gql.dart';
import '../../graphql/queries/__generated__/work_crumb.req.gql.dart';
import '../../graphql/queries/__generated__/work_detail.data.gql.dart';
import '../../graphql/queries/__generated__/work_detail.req.gql.dart';
import '../../graphql/queries/__generated__/work_detail.var.gql.dart';
import '../../graphql/queries/fragments/__generated__/media_file_full.data.gql.dart';
import '../../graphql/query.dart';
import '../../models/media_probe.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/watch_state.dart';
import '../../utils/tracks.dart';
import '../player/decoder_probe.dart';
import '../player/decoders.dart';
import '../player/playback_service.dart';

class UpNext {
  const UpNext(this.work, {this.resume = false, this.rewatch = false});
  final WorkSummary work;
  final bool resume;
  final bool rewatch;
}

class WorkDetailController {
  WorkDetailController({required this.workId, this.server, this.seed}) {
    query = Query(
      _graphql.clientFor(server),
      GWorkDetailReq((b) => b.vars.id = workId),
    );
    _stop = [
      effect(() {
        final data = query.data.value;
        if (data != null) untracked(() => _onDetail(data));
      }),
    ];
    unawaited(_loadDownloadsAllowed());
  }

  Future<void> _loadDownloadsAllowed() async {
    try {
      final data = await _graphql.run(
        GServerInfoReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
        server: server,
      );
      if (!_disposed) downloadsAllowed.value = data.serverInfo.downloadsAllowed;
    } on Exception {
      // Asking again is the download itself, which the server can refuse.
    }
  }

  final String workId;
  final ServerRef? server;
  final WorkSummary? seed;

  GraphqlService get _graphql => getIt<GraphqlService>();
  late final Query<GWorkDetailData, GWorkDetailVars> query;
  late final List<void Function()> _stop;
  bool _loadedChildren = false;
  bool _disposed = false;

  final editionId = signal<String?>(null);
  final filesByEdition = signal<Map<String, List<GMediaFileFull>>>(const {});
  final mediaId = signal<String?>(null);
  final audioChoice = signal<int?>(null);
  final subtitleChoice = signal<int?>(null);
  final overviewExpanded = signal(false);

  /// What the server this title is on says; the button only goes away on a no.
  final downloadsAllowed = signal(true);

  /// The viewer picked; automatic choices stop.
  bool _editionPicked = false;
  bool _versionPicked = false;

  final upNext = signal<UpNext?>(null);
  final seasons = signal<List<WorkSummary>>(const []);
  final selectedSeason = signal<WorkSummary?>(null);
  final episodes = signal<List<WorkSummary>>(const []);
  final loadingEpisodes = signal(false);
  final crumbs = signal<List<WorkSummary>>(const []);

  late final work = computed(() {
    final data = query.data.value?.work;
    if (data == null) return seed;
    return WorkSummary.fromCard(
      data,
      seriesTitle: data.seriesTitle,
      overview: data.overview,
      server: server,
    );
  });

  late final editions = computed(
    () =>
        query.data.value?.editions.toList() ??
        const <GWorkDetailData_editions>[],
  );

  late final edition = computed(() {
    final id = editionId.value;
    for (final item in editions.value) {
      if (item.id == id) return item;
    }
    return editions.value.isEmpty ? null : editions.value.first;
  });

  /// The chosen edition's files as versions, best for this device first.
  late final versions = computed(() {
    final id = edition.value?.id;
    final files = filesByEdition.value[id] ?? const <GMediaFileFull>[];
    final all = [for (final file in files) versionFromFile(file)];
    final profile = getIt<DecoderProbeService>().profile.value;
    if (profile == null) return all;
    final parts = all.map((v) => v.partIndex).toSet().toList()..sort();
    return [
      for (final part in parts)
        ...rankVersions(
          [
            for (final v in all)
              if (v.partIndex == part) v,
          ],
          profile,
          displayPixels: displayPixels(),
        ),
    ];
  });

  late final multiPart = computed(
    () => versions.value.map((v) => v.partIndex).toSet().length > 1,
  );

  late final version = computed(() {
    final id = mediaId.value;
    for (final item in versions.value) {
      if (item.id == id) return item;
    }
    return versions.value.isEmpty ? null : versions.value.first;
  });

  late final mediaFile = computed(() {
    final id = version.value?.id;
    for (final file
        in filesByEdition.value[edition.value?.id] ??
            const <GMediaFileFull>[]) {
      if (file.id == id) return file;
    }
    return null;
  });

  late final audioStreams = computed(
    () => version.value?.probe.audio ?? const <AudioProbe>[],
  );

  late final subtitleTracks = computed(() {
    final file = mediaFile.value;
    return file == null ? const <SubtitleChoice>[] : subtitleChoicesOf(file);
  });

  late final chosenAudio = computed(() {
    final index = audioChoice.value;
    final streams = audioStreams.value;
    return index != null && index < streams.length ? streams[index] : null;
  });

  late final isContainer = computed(() => work.value?.isContainer ?? false);

  void _onDetail(GWorkDetailData data) {
    final first = data.work;
    if (first != null && first.parentId != null && crumbs.value.isEmpty) {
      unawaited(_loadCrumbs(first.parentId!));
    }
    final list = data.editions;
    if (list.isNotEmpty) unawaited(_loadFiles(list.map((e) => e.id).toList()));
    final current = work.value;
    if (current != null && current.isContainer && !_loadedChildren) {
      _loadedChildren = true;
      unawaited(_loadChildren(current));
    }
  }

  Future<void> _loadFiles(List<String> ids) async {
    final missing = ids
        .where((id) => !filesByEdition.value.containsKey(id))
        .toList();
    if (missing.isEmpty) return;
    final answers = await Future.wait(
      missing.map(
        (id) => _graphql
            .run(
              GMediaFilesReq(
                (b) => b
                  ..fetchPolicy = FetchPolicy.CacheFirst
                  ..vars.edition = id,
              ),
              server: server,
            )
            .then<(String, List<GMediaFileFull>)>(
              (data) => (id, data.mediaFiles.toList()),
            )
            .catchError((Object _) => (id, const <GMediaFileFull>[])),
      ),
    );
    if (_disposed) return;
    filesByEdition.value = {
      ...filesByEdition.value,
      for (final (id, files) in answers) id: files,
    };
    await _chooseDefaults(ids);
  }

  Future<void> _chooseDefaults(List<String> ids) async {
    final profile = await getIt<DecoderProbeService>().ready();
    if (_disposed) return;
    final files = filesByEdition.value;
    if (!_editionPicked && !_versionPicked) {
      final choice = pickEdition(
        [
          for (final id in ids)
            (
              id,
              [
                for (final file in files[id] ?? const <GMediaFileFull>[])
                  versionFromFile(file),
              ],
            ),
        ],
        profile,
        displayPixels: displayPixels(),
      );
      if (choice != null) {
        batch(() {
          editionId.value = choice.editionId;
          mediaId.value = choice.best?.id;
        });
      }
    }
    _seedTracks();
  }

  void _seedTracks() {
    final prefs = getIt<SessionService>().playbackPrefs.value;
    batch(() {
      audioChoice.value = preferredAudio(audioStreams.value, prefs);
      subtitleChoice.value = preferredSubtitle(subtitleTracks.value, prefs);
    });
  }

  void chooseEdition(String id) {
    _editionPicked = true;
    _versionPicked = false;
    final profile = getIt<DecoderProbeService>().profile.value;
    final files = filesByEdition.value[id] ?? const <GMediaFileFull>[];
    final part = firstPartOf([for (final file in files) versionFromFile(file)]);
    batch(() {
      editionId.value = id;
      mediaId.value = part.isEmpty
          ? null
          : (profile == null
                    ? part.first
                    : rankVersions(
                        part,
                        profile,
                        displayPixels: displayPixels(),
                      ).first)
                .id;
    });
    _seedTracks();
  }

  void chooseVersion(String id) {
    _versionPicked = true;
    mediaId.value = id;
    _seedTracks();
  }

  Future<void> _loadCrumbs(String parentId) async {
    try {
      final parent = await _crumb(parentId);
      if (parent == null || _disposed) return;
      final grand = parent.parentId == null
          ? null
          : await _crumb(parent.parentId!);
      if (_disposed) return;
      crumbs.value = [?grand, parent];
    } on Exception {
      // Breadcrumbs are a courtesy.
    }
  }

  Future<WorkSummary?> _crumb(String id) async {
    final data = await _graphql.run(
      GWorkCrumbReq(
        (b) => b
          ..fetchPolicy = FetchPolicy.CacheFirst
          ..vars.id = id,
      ),
      server: server,
    );
    final work = data.work;
    return work == null
        ? null
        : WorkSummary.fromCard(
            work,
            seriesTitle: work.seriesTitle,
            server: server,
          );
  }

  /// A show's seasons and what to start with: the episode in progress, then the
  /// first unwatched, else the very first, for a rewatch.
  Future<void> _loadChildren(WorkSummary container) async {
    final playback = getIt<PlaybackService>();
    loadingEpisodes.value = true;
    try {
      final top = await playback.fetchChildren(
        container.libraryId,
        container.id,
        server: server,
      );
      if (_disposed) return;
      final seasonList = top.where((item) => item.kind == 'SEASON').toList();
      final groups = seasonList.isNotEmpty
          ? await Future.wait(
              seasonList.map(
                (s) => playback.fetchChildren(
                  container.libraryId,
                  s.id,
                  server: server,
                ),
              ),
            )
          : [top];
      if (_disposed) return;
      final all = groups.expand((group) => group).toList();
      final started = all.where((item) {
        final data = item.userData;
        return data != null && !data.played && (data.positionMs ?? 0) > 0;
      }).firstOrNull;
      final unwatched = all
          .where((item) => !(item.userData?.played ?? false))
          .firstOrNull;
      final next = started ?? unwatched;
      final focusIndex = next == null || seasonList.isEmpty
          ? 0
          : groups.indexWhere(
              (group) => group.any((item) => item.id == next.id),
            );
      batch(() {
        seasons.value = seasonList;
        selectedSeason.value = seasonList.isEmpty
            ? null
            : seasonList[focusIndex < 0 ? 0 : focusIndex];
        episodes.value = seasonList.isEmpty
            ? top
            : groups[focusIndex < 0 ? 0 : focusIndex];
        upNext.value = next != null
            ? UpNext(next, resume: started != null)
            : all.isNotEmpty
            ? UpNext(all.first, rewatch: true)
            : null;
      });
    } on Exception {
      // The rest of the page still works.
    } finally {
      if (!_disposed) loadingEpisodes.value = false;
    }
  }

  Future<void> chooseSeason(WorkSummary season) async {
    selectedSeason.value = season;
    loadingEpisodes.value = true;
    try {
      final list = await getIt<PlaybackService>().fetchChildren(
        season.libraryId,
        season.id,
        server: server,
      );
      if (_disposed || selectedSeason.value?.id != season.id) return;
      episodes.value = list;
    } on Exception catch (error) {
      getIt<ToastService>().show(error.toString(), kind: ToastKind.error);
    } finally {
      if (!_disposed) loadingEpisodes.value = false;
    }
  }

  void play({bool fromStart = false, int? positionMs}) {
    final current = work.value;
    if (current == null) return;
    unawaited(
      getIt<PlaybackService>().playWork(
        current,
        editionId: _editionPicked ? edition.value?.id : null,
        mediaFileId: _versionPicked || _editionPicked
            ? version.value?.id
            : null,
        audioStream: audioChoice.value,
        subtitleIndex: subtitleChoice.value,
        subtitleChosen: true,
        fromStart: fromStart,
        positionMs: positionMs,
      ),
    );
  }

  Future<void> toggleFavorite() async {
    final current = work.value;
    if (current == null) return;
    final watch = getIt<WatchStateService>();
    final before = watch.of(current);
    final favorite = !before.favorite;
    watch.set(current.id, before.copyWith(favorite: favorite));
    final result = await getIt<ToastService>().attempt(
      () => _graphql.run(
        GSetFavoriteReq(
          (b) => b.vars
            ..id = current.id
            ..favorite = favorite,
        ),
        server: server,
      ),
    );
    if (result == null) watch.set(current.id, before);
  }

  Future<void> togglePlayed() async {
    final current = work.value;
    if (current == null) return;
    final watch = getIt<WatchStateService>();
    final before = watch.of(current);
    final played = !before.played;
    watch.set(current.id, before.copyWith(played: played));
    final ok = await getIt<ToastService>().attempt(
      () => played
          ? _graphql.run(
              GMarkPlayedReq((b) => b.vars.id = current.id),
              server: server,
            )
          : _graphql.run(
              GMarkUnplayedReq((b) => b.vars.id = current.id),
              server: server,
            ),
      success: played ? 'Marked as watched' : 'Marked as unwatched',
    );
    if (ok == null) watch.set(current.id, before);
  }

  void dispose() {
    _disposed = true;
    for (final stop in _stop) {
      stop();
    }
    query.dispose();
  }
}
