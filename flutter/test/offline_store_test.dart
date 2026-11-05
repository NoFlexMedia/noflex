// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/features/downloads/download_models.dart';
import 'package:noflex/features/downloads/downloads_service.dart';
import 'package:noflex/models/work.dart';

OfflineItem item(String id) => OfflineItem(
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

  test(
    'overlapping saves all land, and the newest state is what stays',
    () async {
      final directory = await Directory.systemTemp.createTemp('noflex-store');
      addTearDown(() => directory.delete(recursive: true));
      const channel = MethodChannel('plugins.flutter.io/path_provider');
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(channel, (_) async => directory.path);
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));

      final store = OfflineStore()..items = [item('a')];
      final first = store.save();
      store.items = [item('a'), item('b')];
      await Future.wait([first, store.save(), store.save()]);

      final reloaded = OfflineStore();
      await reloaded.load();
      expect(reloaded.items.map((i) => i.work.id), ['a', 'b']);
    },
  );
}
