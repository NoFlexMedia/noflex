// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/models/work.dart';
import 'package:noflex/utils/merge.dart';

WorkSummary work(String id, {String? tmdb, int? position, ServerRef? server}) =>
    WorkSummary(
      id: id,
      kind: 'MOVIE',
      libraryId: 'l',
      title: id,
      externalIds: tmdb == null ? const [] : [ExternalId('TMDB', tmdb)],
      userData: position == null ? null : WatchData(positionMs: position),
      server: server,
    );

const elsewhere = ServerRef(
  id: 's',
  name: 'Other',
  origin: 'http://other',
  apiKey: 'k',
);

void main() {
  test(
    'one title on two servers becomes one entry, kept from the home server',
    () {
      final merged = mergeRanked([
        [work('home', tmdb: '1')],
        [work('remote', tmdb: '1', server: elsewhere)],
      ]);
      expect(merged.map((w) => w.id), ['home']);
    },
  );

  test('titles without provider IDs never merge', () {
    final merged = mergeRanked([
      [work('a')],
      [work('b', server: elsewhere)],
    ]);
    expect(merged, hasLength(2));
  });

  test('a title on both servers outranks one on either alone', () {
    final merged = mergeRanked([
      [work('solo', tmdb: '1'), work('shared', tmdb: '2')],
      [work('shared2', tmdb: '2', server: elsewhere)],
    ]);
    expect(merged.first.id, 'shared');
  });

  test('continue watching keeps whichever server is further along', () {
    final merged = mergeContinueWatching([
      [work('home', tmdb: '1', position: 100)],
      [work('remote', tmdb: '1', position: 900, server: elsewhere)],
    ]);
    expect(merged.single.id, 'remote');
  });
}
