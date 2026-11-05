// SPDX-License-Identifier: AGPL-3.0-only
//
// Combining lists from several servers. Scores aren't comparable across
// servers, so lists merge by rank, the same way hybrid search fuses its own.

import '../models/work.dart';

const _rrfK = 60;

Iterable<String> _idKeys(List<ExternalId> ids) => ids
    .where((id) => id.kind != 'CUSTOM')
    .map((id) => '${id.kind}:${id.value}');

/// What makes two works the same title: a shared provider ID. Paths and other
/// server-local IDs never count, and unidentified works never merge, since a
/// wrong merge is worse than a duplicate. Episodes rarely have IDs of their
/// own, so their series' IDs and their numbers identify them too.
List<String> providerKeys(WorkSummary work) {
  final own = _idKeys(work.externalIds).toList();
  if (work.parentIndex == null || work.indexNumber == null) return own;
  return [
    ...own,
    ..._idKeys(work.seriesExternalIds)
        .map((key) => '$key/S${work.parentIndex}E${work.indexNumber}'),
  ];
}

class _Entry {
  _Entry(this.work, this.order);
  WorkSummary work;
  final int order;
  double score = 0;
}

/// Reciprocal rank fusion of [lists], best first. Works matching by provider
/// ID become one entry, keeping the version from the earliest list unless
/// [prefer] picks a later one, so the home server wins a tie.
List<WorkSummary> mergeRanked(
  List<List<WorkSummary>> lists, {
  bool Function(WorkSummary candidate, WorkSummary current)? prefer,
}) {
  final entries = <_Entry>[];
  final byKey = <String, _Entry>{};
  for (final list in lists) {
    for (var rank = 0; rank < list.length; rank++) {
      final work = list[rank];
      final keys = providerKeys(work);
      _Entry? known;
      for (final key in keys) {
        known ??= byKey[key];
      }
      final entry = known ?? _Entry(work, entries.length);
      if (known == null) {
        entries.add(entry);
      } else if (prefer?.call(work, known.work) ?? false) {
        known.work = work;
      }
      entry.score += 1 / (_rrfK + rank + 1);
      for (final key in keys) {
        byKey[key] = entry;
      }
    }
  }
  entries.sort((a, b) {
    final byScore = b.score.compareTo(a.score);
    return byScore != 0 ? byScore : a.order.compareTo(b.order);
  });
  return [for (final entry in entries) entry.work];
}

/// A title in progress on any server appears once, where it was watched last.
List<WorkSummary> mergeContinueWatching(List<List<WorkSummary>> lists) =>
    mergeRanked(
      lists,
      prefer: (candidate, current) =>
          (candidate.userData?.positionMs ?? 0) >
          (current.userData?.positionMs ?? 0),
    );
