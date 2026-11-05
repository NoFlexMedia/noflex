// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/queries/__generated__/home.data.gql.dart';
import '../../graphql/queries/__generated__/home.req.gql.dart';
import '../../graphql/query.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../../utils/images.dart';
import '../../utils/merge.dart';

class HomeShelves {
  const HomeShelves({
    this.continueWatching = const [],
    this.recentlyAdded = const [],
    this.recentlyPlayed = const [],
    this.recommendations = const [],
  });

  final List<WorkSummary> continueWatching;
  final List<WorkSummary> recentlyAdded;
  final List<WorkSummary> recentlyPlayed;
  final List<WorkSummary> recommendations;

  bool get isEmpty => continueWatching.isEmpty && recentlyAdded.isEmpty;
}

HomeShelves _shelvesOf(GHomeData data, ServerRef? server) => HomeShelves(
  continueWatching: [
    for (final work in data.continueWatching.nodes)
      WorkSummary.fromCard(
        work,
        seriesTitle: work.seriesTitle,
        seriesExternalIds: [
          for (final id in work.seriesExternalIds)
            ExternalId(id.kind.name, id.value),
        ],
        server: server,
      ),
  ],
  recentlyAdded: [
    for (final work in data.recentlyAdded.nodes)
      WorkSummary.fromCard(work, overview: work.overview, server: server),
  ],
  recentlyPlayed: [
    for (final work in data.recentlyPlayed.nodes)
      WorkSummary.fromCard(work, seriesTitle: work.seriesTitle, server: server),
  ],
  recommendations: [
    for (final work in data.recommendations.nodes)
      WorkSummary.fromCard(work, server: server),
  ],
);

/// The home screen's data: this server's shelves right away, other servers'
/// merged in as they answer, so a slow one never holds up the page.
class HomeService {
  HomeService() {
    _graphql = getIt<GraphqlService>();
    _session = getIt<SessionService>();
    _start();
  }

  late final GraphqlService _graphql;
  late final SessionService _session;
  Query<GHomeData, dynamic>? _query;
  final _elsewhere = signal<List<HomeShelves>>(const []);

  /// Servers that didn't answer the last time they were asked.
  final unreachable = signal<List<String>>(const []);

  Query<GHomeData, dynamic> get query => _query!;

  late final own = computed(() {
    final data = _query?.data.value;
    return data == null ? null : _shelvesOf(data, null);
  });

  late final shelves = computed(() {
    final mine = own.value;
    if (mine == null) return null;
    final others = _elsewhere.value;
    if (others.isEmpty) return mine;
    return HomeShelves(
      continueWatching: mergeContinueWatching([
        mine.continueWatching,
        for (final other in others) other.continueWatching,
      ]),
      recentlyAdded: mergeRanked([
        mine.recentlyAdded,
        for (final other in others) other.recentlyAdded,
      ]),
      recentlyPlayed: mine.recentlyPlayed,
      recommendations: mergeRanked([
        mine.recommendations,
        for (final other in others) other.recommendations,
      ]),
    );
  });

  /// The hero's slides: this server's own, top-level, with art if any has it.
  late final featured = computed(() {
    const topLevel = {'MOVIE', 'SERIES', 'ALBUM', 'AUDIO_BOOK'};
    final pool = (own.value?.recentlyAdded ?? const <WorkSummary>[])
        .where((work) => topLevel.contains(work.kind))
        .toList();
    final withArt = pool
        .where((work) => pickImage(work.images, ArtKind.backdrop) != null)
        .toList();
    return (withArt.isNotEmpty ? withArt : pool).take(6).toList();
  });

  void _start() {
    _query?.dispose();
    _query = Query(_graphql.client, GHomeReq());
    unawaited(_askOthers());
  }

  void refresh() {
    _query?.refetch();
    unawaited(_askOthers());
  }

  Future<void> _askOthers() async {
    final servers = _session.connectedServers.value;
    if (servers.isEmpty) {
      batch(() {
        _elsewhere.value = const [];
        unreachable.value = const [];
      });
      return;
    }
    final answers = <HomeShelves>[];
    final missing = <String>[];
    await Future.wait([
      for (final server in servers)
        _graphql
            .run(
              GHomeReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
              server: server,
              timeout: const Duration(seconds: 5),
            )
            .then((data) => answers.add(_shelvesOf(data, server)))
            .catchError((Object _) {
              missing.add(server.name);
              return answers;
            }),
    ]);
    batch(() {
      _elsewhere.value = answers;
      unreachable.value = missing;
    });
  }
}
