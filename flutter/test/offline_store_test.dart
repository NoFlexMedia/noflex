// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/downloads/download_models.dart';
import 'package:noflex/features/downloads/downloads_service.dart';
import 'package:noflex/models/work.dart';

OfflineItem item(String id, {String account = 'vera'}) => OfflineItem(
  accountId: account,
  work: WorkSummary(id: id, kind: 'MOVIE', libraryId: 'l', title: id),
  partIndex: 0,
  origin: OfflineOrigin.manual,
  status: OfflineStatus.complete,
  sizeBytes: 1,
  fileName: '$id.mkv',
  addedAt: DateTime(2026),
);

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late Directory directory;
  setUp(() async {
    directory = await Directory.systemTemp.createTemp('noflex-store');
    addTearDown(() => directory.delete(recursive: true));
    const channel = MethodChannel('plugins.flutter.io/path_provider');
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(channel, (_) async => directory.path);
    addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
  });

  test(
    'overlapping saves all land, and the newest state is what stays',
    () async {
      final store = OfflineStore('vera')..items = [item('a')];
      final first = store.save();
      store.items = [item('a'), item('b')];
      await Future.wait([first, store.save(), store.save()]);

      final reloaded = OfflineStore('vera');
      await reloaded.load();
      expect(reloaded.items.map((i) => i.work.id), ['a', 'b']);
    },
  );

  test('each account keeps a manifest of its own', () async {
    final vera = OfflineStore('vera')
      ..items = [item('a')]
      ..held = ['vera-a-0'];
    final kid = OfflineStore('kid')..items = [item('a', account: 'kid')];
    await Future.wait([vera.save(), kid.save()]);
    // The same title for two people is two transfers and two files.
    expect(vera.items.single.taskId, isNot(kid.items.single.taskId));
    expect(
      await mediaPathOf(vera.items.single),
      isNot(await mediaPathOf(kid.items.single)),
    );

    await kid.delete();
    expect(await OfflineStore.accountIds(), ['vera']);
    final reloaded = OfflineStore('vera');
    await reloaded.load();
    expect(reloaded.items.single.accountId, 'vera');
    expect(reloaded.held, ['vera-a-0']);

    // Nobody active holds nothing, and writes nowhere.
    final nobody = OfflineStore(null)..items = [item('b')];
    await nobody.save();
    expect(await OfflineStore.accountIds(), ['vera']);
  });
}
