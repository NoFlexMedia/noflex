// SPDX-License-Identifier: AGPL-3.0-only
//
// Titles still being indexed. Artwork asks for what it's showing to be watched,
// and one request covers them all. Lists keep the answer they were fetched with,
// so cards read through here, as they do through WatchStateService.

import 'dart:async';

import 'package:ferry/ferry.dart' show FetchPolicy;
import 'package:signals_flutter/signals_flutter.dart';

import '../di.dart';
import '../graphql/queries/__generated__/works_by_id.req.gql.dart';
import '../models/work.dart';
import 'graphql.dart';

const _pollEvery = Duration(seconds: 4);

/// A longer list takes turns.
const _batch = 30;

const indexingLabels = {
  'ADDING': 'Adding to library',
  'FINDING_DETAILS': 'Finding details',
};

class IndexingService {
  final _answers = signal<Map<String, WorkSummary?>>(const {});
  final _watching = <String, int>{};
  Timer? _timer;
  var _offset = 0;

  /// Reads reactively, so call this inside a SignalBuilder. A linked server's
  /// title can't be followed from here, so it's left as its list had it.
  WorkSummary latest(WorkSummary work) {
    if (work.indexing == null || work.server != null) return work;
    final answers = _answers.value;
    return answers.containsKey(work.id)
        ? work.updatedBy(answers[work.id])
        : work;
  }

  /// Keeps [id] fresh until the returned function is called.
  void Function() watch(String id) {
    _watching[id] = (_watching[id] ?? 0) + 1;
    _timer ??= Timer.periodic(_pollEvery, (_) => unawaited(_refresh()));
    return () {
      final left = (_watching[id] ?? 1) - 1;
      if (left > 0) {
        _watching[id] = left;
      } else {
        _watching.remove(id);
      }
      if (_watching.isEmpty) {
        _timer?.cancel();
        _timer = null;
      }
    };
  }

  Future<void> _refresh() async {
    final ids = _watching.keys.toList();
    if (_offset >= ids.length) _offset = 0;
    final turn = ids.skip(_offset).take(_batch).toList();
    _offset += _batch;
    if (turn.isEmpty) return;
    try {
      final data = await getIt<GraphqlService>().run(
        GWorksByIdReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NetworkOnly
            ..vars.ids.replace(turn),
        ),
      );
      final found = {
        for (final card in data.worksById) card.id: WorkSummary.fromCard(card),
      };
      _answers.value = {
        ..._answers.value,
        for (final id in turn) id: found[id],
      };
    } on Exception {
      // The next tick asks again.
    }
  }

  void dispose() {
    _timer?.cancel();
    _timer = null;
  }
}
