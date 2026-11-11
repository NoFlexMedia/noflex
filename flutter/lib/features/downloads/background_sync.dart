// SPDX-License-Identifier: AGPL-3.0-only
//
// Smart downloads while the app isn't open. WorkManager runs this periodically on
// Android; on iOS BGTaskScheduler runs it when the system feels like it, which may
// be rarely. Everything it enqueues is then the system's to finish. No get_it
// here: this is its own isolate, with none of the app's services.

import 'dart:async';
import 'dart:io';

import 'package:background_downloader/background_downloader.dart';
import 'package:ferry/ferry.dart';
import 'package:flutter/widgets.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:workmanager/workmanager.dart';

import '../../graphql/queries/__generated__/smart_downloads.req.gql.dart';
import '../../services/device_identity.dart';
import '../../services/graphql.dart';
import '../../services/tokens.dart';
import 'download_models.dart';
import 'downloads_service.dart';
import 'smart_planner.dart';

const smartSyncTask = 'dev.noflex.app.smartDownloads';
const _targetKey = 'noflex.downloadDirectory';

bool get _supported => Platform.isAndroid || Platform.isIOS;

Future<void> scheduleSmartSync() async {
  if (!_supported) return;
  await Workmanager().initialize(smartSyncDispatcher);
  await Workmanager().registerPeriodicTask(
    smartSyncTask,
    smartSyncTask,
    frequency: const Duration(hours: 1),
    existingWorkPolicy: ExistingPeriodicWorkPolicy.keep,
    constraints: Constraints(
      networkType: NetworkType.connected,
      requiresBatteryNotLow: true,
    ),
  );
}

@pragma('vm:entry-point')
void smartSyncDispatcher() {
  Workmanager().executeTask((task, input) async {
    WidgetsFlutterBinding.ensureInitialized();
    try {
      // The OS only allows so long, and a stuck plugin call must not use it up.
      await runSmartSyncOnce().timeout(const Duration(minutes: 5));
    } catch (error) {
      debugPrint('smart sync: $error');
    }
    // Always true: a retry would only ask the same question again, and the next
    // period comes regardless.
    return true;
  });
}

Future<void> runSmartSyncOnce() async {
  final prefs = await SharedPreferences.getInstance();
  await prefs.reload();
  // An app on screen reconciles for itself. Two copies of the manifest would
  // overwrite each other, and two refreshes of one token would look like replay
  // to the server, which ends the session.
  final stamp = prefs.getInt(foregroundStampKey) ?? 0;
  if (DateTime.now().millisecondsSinceEpoch - stamp <
      const Duration(minutes: 2).inMilliseconds) {
    return;
  }
  final tokens = TokenStore();
  await tokens.load();
  // Whoever is active, if anyone: with the app on "Who's watching?" there's
  // nobody to fetch for, and a locked session would only refuse.
  final account = tokens.active;
  if (account == null || tokens.origin.value == null) return;
  final deviceId = prefs.getString(deviceIdKey(account.userId));
  if (deviceId == null) return;

  final graphql = GraphqlService(tokens);
  final store = OfflineStore(account.userId);
  await store.load();

  final data = await graphql.run(
    GSmartDownloadsReq(
      (b) => b
        ..fetchPolicy = FetchPolicy.NetworkOnly
        ..vars.device = deviceId,
    ),
  );
  final wanted = [
    for (final download in data.smartDownloads) wantedFromDownload(download),
  ];
  // The server answers empty with smart downloads off, which would read as
  // "nothing wanted" and evict everything. Leave that decision to the app.
  if (wanted.isEmpty) return;
  final plan = planSmartDownloads(
    enabled: true,
    wanted: wanted,
    items: store.items,
  );
  if (plan.isEmpty) return;

  final directory = chosenDirectory(prefs.getString(_targetKey), account);
  for (final item in plan.evict) {
    await FileDownloader().cancelTaskWithId(item.taskId);
    await deleteOfflineFiles(item);
  }
  final kept = [
    for (final item in store.items)
      if (!plan.evict.any((evicted) => evicted.key == item.key)) item,
  ];
  final added = <OfflineItem>[];
  for (final file in plan.fetch) {
    final item = itemFor(
      file,
      OfflineOrigin.smart,
      accountId: account.userId,
      directory: directory,
    );
    await enqueueTransfer(item, tokens.absolute(file.url), wifiOnly: true);
    await enqueueExtras(item, file, absolute: tokens.absolute);
    added.add(item);
  }
  store.items = [
    for (final item in kept)
      if (!added.any((next) => next.key == item.key)) item,
    ...added,
  ];
  await store.save();
}
