// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/downloads/download_models.dart';
import 'package:noflex/features/downloads/smart_planner.dart';
import 'package:noflex/models/work.dart';

WorkSummary work(String id) =>
    WorkSummary(id: id, kind: 'EPISODE', libraryId: 'l', title: id);

WantedFile wanted(String id) =>
    WantedFile(work: work(id), partIndex: 0, url: '/stream/$id', sizeBytes: 10);

OfflineItem item(
  String id, {
  OfflineOrigin origin = OfflineOrigin.smart,
  OfflineStatus status = OfflineStatus.complete,
}) => OfflineItem(
  accountId: 'viewer',
  work: work(id),
  partIndex: 0,
  origin: origin,
  status: status,
  sizeBytes: 10,
  fileName: '$id.mkv',
  addedAt: DateTime(2026),
);

void main() {
  test('fetches what’s missing, in the server’s order', () {
    final plan = planSmartDownloads(
      enabled: true,
      wanted: [wanted('e2'), wanted('e3'), wanted('e4')],
      items: [item('e3')],
    );
    expect(plan.fetch.map((f) => f.work.id), ['e2', 'e4']);
    expect(plan.evict, isEmpty);
  });

  test(
    'evicts what the server stopped wanting, such as a finished episode',
    () {
      final plan = planSmartDownloads(
        enabled: true,
        wanted: [wanted('e2')],
        items: [item('e1'), item('e2')],
      );
      expect(plan.evict.map((i) => i.work.id), ['e1']);
      expect(plan.fetch, isEmpty);
    },
  );

  test('never evicts what the viewer chose to keep', () {
    final plan = planSmartDownloads(
      enabled: true,
      wanted: const [],
      items: [item('film', origin: OfflineOrigin.manual)],
    );
    expect(plan.isEmpty, isTrue);
  });

  test('retries a failed transfer, which gets the fresh URL just received', () {
    final plan = planSmartDownloads(
      enabled: true,
      wanted: [wanted('e2')],
      items: [item('e2', status: OfflineStatus.failed)],
    );
    expect(plan.fetch.map((f) => f.work.id), ['e2']);
  });

  test('turning it off stops transfers but keeps finished files', () {
    final plan = planSmartDownloads(
      enabled: false,
      wanted: [wanted('e9')],
      items: [
        item('done'),
        item('running', status: OfflineStatus.downloading),
        item(
          'mine',
          origin: OfflineOrigin.manual,
          status: OfflineStatus.downloading,
        ),
      ],
    );
    expect(plan.fetch, isEmpty);
    expect(plan.evict.map((i) => i.work.id), ['running']);
  });
}
