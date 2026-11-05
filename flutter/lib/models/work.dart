// SPDX-License-Identifier: AGPL-3.0-only
//
// A work as the UI draws it. Generated GraphQL types differ per operation and
// can't carry a linked server or live in the offline manifest, so everything
// is mapped to this once, at the seam, right where a query's answer arrives.

import '../graphql/__generated__/schema.schema.gql.dart';
import '../graphql/queries/fragments/__generated__/work_card.data.gql.dart';
import '../utils/images.dart';

class ExternalId {
  const ExternalId(this.kind, this.value);
  factory ExternalId.fromJson(Map<String, dynamic> json) =>
      ExternalId(json['kind'] as String, json['value'] as String);
  final String kind;
  final String value;
  Map<String, dynamic> toJson() => {'kind': kind, 'value': value};
}

class WatchData {
  const WatchData({
    this.played = false,
    this.positionMs,
    this.favorite = false,
  });

  factory WatchData.fromJson(Map<String, dynamic> json) => WatchData(
    played: json['played'] as bool? ?? false,
    positionMs: json['positionMs'] as int?,
    favorite: json['favorite'] as bool? ?? false,
  );

  final bool played;
  final int? positionMs;
  final bool favorite;

  WatchData copyWith({bool? played, int? positionMs, bool? favorite}) =>
      WatchData(
        played: played ?? this.played,
        positionMs: positionMs ?? this.positionMs,
        favorite: favorite ?? this.favorite,
      );

  Map<String, dynamic> toJson() => {
    'played': played,
    'positionMs': positionMs,
    'favorite': favorite,
  };
}

/// A linked server a work came from, reduced to what requests need.
class ServerRef {
  const ServerRef({
    required this.id,
    required this.name,
    required this.origin,
    required this.apiKey,
  });
  final String id;
  final String name;
  final String origin;
  final String apiKey;
}

const _containerKinds = {'SERIES', 'SEASON', 'ARTIST', 'ALBUM'};

class WorkSummary {
  const WorkSummary({
    required this.id,
    required this.kind,
    required this.libraryId,
    required this.title,
    this.parentId,
    this.year,
    this.communityRating,
    this.indexNumber,
    this.indexEnd,
    this.parentIndex,
    this.runtimeMs,
    this.externalIds = const [],
    this.seriesExternalIds = const [],
    this.images = const [],
    this.userData,
    this.seriesTitle,
    this.overview,
    this.server,
  });

  factory WorkSummary.fromCard(
    GWorkCard card, {
    String? seriesTitle,
    String? overview,
    List<ExternalId> seriesExternalIds = const [],
    ServerRef? server,
  }) => WorkSummary(
    id: card.id,
    kind: card.kind.name,
    parentId: card.parentId,
    libraryId: card.libraryId,
    title: card.title,
    year: card.year,
    communityRating: card.communityRating,
    indexNumber: card.indexNumber,
    indexEnd: card.indexEnd,
    parentIndex: card.parentIndex,
    runtimeMs: card.runtimeMs,
    externalIds: [
      for (final id in card.externalIds) ExternalId(id.kind.name, id.value),
    ],
    seriesExternalIds: seriesExternalIds,
    images: [
      for (final image in card.images)
        ArtImage(image.kind.name, _absolute(image.url, server)),
    ],
    userData: card.userData == null
        ? null
        : WatchData(
            played: card.userData!.played,
            positionMs: card.userData!.positionMs,
            favorite: card.userData!.favorite,
          ),
    seriesTitle: seriesTitle,
    overview: overview,
    server: server,
  );

  factory WorkSummary.fromJson(Map<String, dynamic> json) => WorkSummary(
    id: json['id'] as String,
    kind: json['kind'] as String,
    parentId: json['parentId'] as String?,
    libraryId: json['libraryId'] as String,
    title: json['title'] as String,
    year: json['year'] as int?,
    communityRating: (json['communityRating'] as num?)?.toDouble(),
    indexNumber: json['indexNumber'] as int?,
    indexEnd: json['indexEnd'] as int?,
    parentIndex: json['parentIndex'] as int?,
    runtimeMs: json['runtimeMs'] as int?,
    images: [
      for (final image in json['images'] as List? ?? const [])
        ArtImage.fromJson((image as Map).cast<String, dynamic>()),
    ],
    userData: json['userData'] == null
        ? null
        : WatchData.fromJson((json['userData'] as Map).cast<String, dynamic>()),
    seriesTitle: json['seriesTitle'] as String?,
    overview: json['overview'] as String?,
  );

  final String id;

  /// The `WorkKind` name; a string so a work can sit in the offline manifest.
  final String kind;
  final String? parentId;
  final String libraryId;
  final String title;
  final int? year;
  final double? communityRating;
  final int? indexNumber;
  final int? indexEnd;
  final int? parentIndex;
  final int? runtimeMs;
  final List<ExternalId> externalIds;
  final List<ExternalId> seriesExternalIds;
  final List<ArtImage> images;
  final WatchData? userData;
  final String? seriesTitle;
  final String? overview;

  /// Null on the home server.
  final ServerRef? server;

  bool get isContainer => _containerKinds.contains(kind);
  bool get isEpisode => kind == GWorkKind.EPISODE.name;

  /// Where a list asks for the series, an episode leads with it.
  String get heading => isEpisode ? (seriesTitle ?? title) : title;

  /// Unmatched episodes are only titled with their number.
  String get episodeTitle =>
      seriesTitle != null &&
          !RegExp(r'^episode \d+$', caseSensitive: false).hasMatch(title)
      ? title
      : '';

  /// "S1 · E2" for an episode, the year otherwise.
  String get subtitle {
    if (isEpisode && indexNumber != null) {
      final episode = indexEnd != null
          ? 'E$indexNumber–$indexEnd'
          : 'E$indexNumber';
      return parentIndex != null ? 'S$parentIndex · $episode' : episode;
    }
    return year?.toString() ?? '';
  }

  WorkSummary withUserData(WatchData data) => WorkSummary(
    id: id,
    kind: kind,
    parentId: parentId,
    libraryId: libraryId,
    title: title,
    year: year,
    communityRating: communityRating,
    indexNumber: indexNumber,
    indexEnd: indexEnd,
    parentIndex: parentIndex,
    runtimeMs: runtimeMs,
    externalIds: externalIds,
    seriesExternalIds: seriesExternalIds,
    images: images,
    userData: data,
    seriesTitle: seriesTitle,
    overview: overview,
    server: server,
  );

  /// For the offline manifest. Image URLs stay as they are; the manifest keeps
  /// its own local copy of the art.
  Map<String, dynamic> toJson() => {
    'id': id,
    'kind': kind,
    'parentId': parentId,
    'libraryId': libraryId,
    'title': title,
    'year': year,
    'communityRating': communityRating,
    'indexNumber': indexNumber,
    'indexEnd': indexEnd,
    'parentIndex': parentIndex,
    'runtimeMs': runtimeMs,
    'images': [for (final image in images) image.toJson()],
    'userData': userData?.toJson(),
    'seriesTitle': seriesTitle,
    'overview': overview,
  };
}

/// Responses use paths relative to the server that sent them.
String _absolute(String url, ServerRef? server) =>
    server != null && url.startsWith('/') ? '${server.origin}$url' : url;
