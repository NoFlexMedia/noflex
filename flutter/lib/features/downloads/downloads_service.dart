// SPDX-License-Identifier: AGPL-3.0-only
//
// Offline titles. background_downloader owns the transfers, natively: URLSession
// background tasks on iOS and macOS, WorkManager on Android, so they carry on with
// the app suspended or gone. This keeps the manifest of what's held, mirrors
// transfer updates into signals, and reconciles with the server's smart list.
//
// Downloads belong to an account. Each person signed in on the device has a
// manifest and a folder of their own, and this holds the active one's: the
// downloader is shared, so a transfer says whose it is in its id, and an update
// for anyone else's is left for their manifest to catch up on when they're back.
//
// iOS gives no guarantee about when an app runs in the background, so everything
// wanted is handed to the system at once rather than one at a time: the transfers
// then finish whether or not Dart ever wakes. See AGENTS.md "Downloads".

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:background_downloader/background_downloader.dart';
import 'package:ferry/ferry.dart';
import 'package:flutter/widgets.dart' show AppLifecycleListener;
import 'package:flutter/foundation.dart';
import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/create_export_job.req.gql.dart';
import '../../graphql/queries/__generated__/device_download.req.gql.dart';
import '../../graphql/queries/__generated__/mark_played.req.gql.dart';
import '../../graphql/queries/__generated__/register_device.req.gql.dart';
import '../../graphql/queries/__generated__/report_progress.req.gql.dart';
import '../../graphql/queries/__generated__/smart_downloads.req.gql.dart';
import '../../graphql/queries/__generated__/update_device.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/download_fields.data.gql.dart';
import '../../models/work.dart';
import '../../services/device_identity.dart';
import '../../services/graphql.dart';
import '../../services/network_service.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
import '../player/capabilities.dart';
import '../player/decoder_probe.dart';
import '../player/decoders.dart';
import 'download_models.dart';
import 'smart_planner.dart';

const _mediaGroup = 'noflex.media';
const _extrasGroup = 'noflex.extras';
const _storageDir = 'offline';
const _targetKey = 'noflex.downloadDirectory';
const _manifestName = 'manifest.json';

/// Stamped while the app is on screen, so the headless sync knows to stay out.
const foregroundStampKey = 'noflex.foregroundAt';

/// Where [accountId]'s downloads live, under the app's own storage.
String offlineDirectory(String accountId) => p.join(_storageDir, accountId);

/// The folder for [account]'s downloads inside one the viewer chose, or null
/// for the app's own storage. Named for people browsing it, with enough of the
/// id that two people of one name don't share it.
String? chosenDirectory(String? target, StoredAccount account) => target == null
    ? null
    : p.join(
        target,
        safeFileName(
          '${account.name} [${account.userId.substring(account.userId.length - 8)}]',
        ),
      );

/// One account's offline manifest on disk, shared by the app and the headless
/// sync. With no account it holds nothing and saves nowhere.
class OfflineStore {
  OfflineStore(this.accountId);

  final String? accountId;
  List<OfflineItem> items = const [];
  List<PendingReport> reports = const [];

  /// Transfers stopped when their profile was locked, to start again when
  /// it's opened.
  List<String> held = const [];

  static Future<Directory> _root() async {
    final base = await getApplicationSupportDirectory();
    return Directory(p.join(base.path, _storageDir));
  }

  /// Every account with a manifest here, signed in or not.
  static Future<List<String>> accountIds() async {
    final root = await _root();
    if (!root.existsSync()) return const [];
    return [
      for (final entry in root.listSync())
        if (entry is Directory &&
            File(p.join(entry.path, _manifestName)).existsSync())
          p.basename(entry.path),
    ];
  }

  Future<File> _file() async {
    final directory = Directory(p.join((await _root()).path, accountId));
    await directory.create(recursive: true);
    return File(p.join(directory.path, _manifestName));
  }

  Future<void> load() async {
    if (accountId == null) return;
    try {
      final file = await _file();
      if (!file.existsSync()) return;
      final json =
          jsonDecode(await file.readAsString()) as Map<String, dynamic>;
      items = [
        for (final item in json['items'] as List? ?? const [])
          OfflineItem.fromJson((item as Map).cast<String, dynamic>()),
      ];
      reports = [
        for (final report in json['reports'] as List? ?? const [])
          PendingReport.fromJson((report as Map).cast<String, dynamic>()),
      ];
      held = [...(json['held'] as List? ?? const []).cast<String>()];
    } catch (error) {
      debugPrint('offline manifest unreadable: $error');
    }
  }

  Future<void> _lastSave = Future.value();

  /// One at a time: they share a temp file, which the first to finish would
  /// rename away from the rest. Each writes what's current when its turn comes.
  Future<void> save() =>
      _lastSave = _lastSave.catchError((Object _) {}).then((_) => _write());

  Future<void> _write() async {
    if (accountId == null || _deleted) return;
    final file = await _file();
    final temp = File('${file.path}.tmp');
    await temp.writeAsString(
      jsonEncode({
        'items': [for (final item in items) item.toJson()],
        'reports': [for (final report in reports) report.toJson()],
        'held': held,
      }),
    );
    await temp.rename(file.path);
  }

  bool _deleted = false;

  /// Remove the manifest and the folder it's in, once its files are gone. A
  /// save still waiting writes nothing.
  Future<void> delete() async {
    if (accountId == null) return;
    _deleted = true;
    await _lastSave.catchError((Object _) {});
    final directory = Directory(p.join((await _root()).path, accountId));
    if (directory.existsSync()) await directory.delete(recursive: true);
  }
}

String safeFileName(String value) => value
    .replaceAll(RegExp(r'[<>:"/\\|?*\x00-\x1f]'), ' ')
    .replaceAll(RegExp(r'\s+'), ' ')
    .trim();

/// A readable name, for folders people browse themselves.
String offlineFileName(WorkSummary work, int partIndex) {
  final title = safeFileName(
    work.isEpisode && work.seriesTitle != null
        ? '${work.seriesTitle} ${work.subtitle.replaceAll(' · ', '')}'
        : work.title,
  );
  final part = partIndex > 0 ? ' part ${partIndex + 1}' : '';
  return '$title$part [${work.id.substring(work.id.length - 8)}].mkv';
}

/// Enqueues the transfer behind an item. Also used headlessly.
Future<DownloadTask> enqueueTransfer(
  OfflineItem item,
  String url, {
  required bool wifiOnly,
}) async {
  final task = DownloadTask(
    taskId: item.taskId,
    group: _mediaGroup,
    url: url,
    filename: item.fileName,
    directory: item.directory ?? offlineDirectory(item.accountId),
    baseDirectory: item.directory != null
        ? BaseDirectory.root
        : BaseDirectory.applicationSupport,
    updates: Updates.statusAndProgress,
    requiresWiFi: wifiOnly,
    allowPause: true,
    retries: 5,
    displayName: item.work.heading,
  );
  await FileDownloader().enqueue(task);
  return task;
}

Future<void> enqueueExtras(
  OfflineItem item,
  WantedFile file, {
  required String Function(String) absolute,
}) async {
  final poster = item.posterFile;
  final posterUrl =
      file.work.images
          .where((image) => image.kind == 'PRIMARY')
          .firstOrNull
          ?.url ??
      (file.work.images.isEmpty ? null : file.work.images.first.url);
  final tasks = <DownloadTask>[
    if (poster != null && posterUrl != null)
      DownloadTask(
        taskId: '${item.taskId}-poster',
        group: _extrasGroup,
        url: '${absolute(posterUrl)}&w=480',
        filename: poster,
        directory: offlineDirectory(item.accountId),
        baseDirectory: BaseDirectory.applicationSupport,
        updates: Updates.none,
        retries: 3,
      ),
    for (final subtitle in file.subtitles)
      DownloadTask(
        taskId: '${item.taskId}-sub-${subtitle.index}',
        group: _extrasGroup,
        url: absolute(subtitle.url),
        filename: '${item.key}-${subtitle.index}.vtt',
        directory: offlineDirectory(item.accountId),
        baseDirectory: BaseDirectory.applicationSupport,
        updates: Updates.none,
        retries: 3,
      ),
  ];
  for (final task in tasks) {
    await FileDownloader().enqueue(task);
  }
}

OfflineItem itemFor(
  WantedFile file,
  OfflineOrigin origin, {
  required String accountId,
  String? directory,
}) => OfflineItem(
  accountId: accountId,
  work: file.work,
  partIndex: file.partIndex,
  origin: origin,
  status: OfflineStatus.queued,
  sizeBytes: file.sizeBytes,
  fileName: offlineFileName(file.work, file.partIndex),
  addedAt: DateTime.now(),
  directory: directory,
  serverId: file.work.server?.id,
  optimized: file.optimized,
  // What's been watched so far, so an offline resume starts in the right place.
  positionMs: file.work.userData?.positionMs ?? 0,
  played: file.work.userData?.played ?? false,
  posterFile: file.work.images.isEmpty ? null : '${file.key}-poster.jpg',
  subtitles: [
    for (final subtitle in file.subtitles)
      OfflineSubtitle(
        index: subtitle.index,
        fileName: '${file.key}-${subtitle.index}.vtt',
        language: subtitle.language,
        title: subtitle.title,
        forced: subtitle.forced,
        isDefault: subtitle.isDefault,
      ),
  ],
);

WantedFile wantedFromDownload(GDownloadFields download, {ServerRef? server}) {
  final work = download.work;
  return WantedFile(
    work: WorkSummary.fromCard(
      work,
      seriesTitle: work.seriesTitle,
      overview: work.overview,
      server: server,
    ),
    partIndex: download.partIndex,
    url: download.url,
    sizeBytes: download.sizeBytes,
    optimized: download.optimized,
    subtitles: [
      for (final subtitle in download.subtitles)
        WantedSubtitle(
          index: subtitle.index,
          url: subtitle.url,
          language: subtitle.language,
          title: subtitle.title,
          forced: subtitle.forced,
          isDefault: subtitle.Gdefault,
        ),
    ],
  );
}

Future<String?> mediaPathOf(OfflineItem item) async {
  if (item.directory != null) return p.join(item.directory!, item.fileName);
  final base = await getApplicationSupportDirectory();
  return p.join(base.path, offlineDirectory(item.accountId), item.fileName);
}

Future<void> deleteOfflineFiles(OfflineItem item) async {
  final base = await getApplicationSupportDirectory();
  final paths = [
    await mediaPathOf(item),
    if (item.posterFile != null)
      p.join(base.path, offlineDirectory(item.accountId), item.posterFile!),
    for (final subtitle in item.subtitles)
      p.join(base.path, offlineDirectory(item.accountId), subtitle.fileName),
  ];
  for (final path in paths) {
    try {
      if (path != null) await File(path).delete();
    } on FileSystemException {
      // Already gone.
    }
  }
}

class DownloadsService {
  var _store = OfflineStore(null);
  StreamSubscription<TaskUpdate>? _updates;
  Timer? _periodic;
  Timer? _saveDebounce;
  Timer? _stamp;
  AppLifecycleListener? _lifecycle;
  void Function()? _stopWatching;
  bool _reconciling = false;
  String _supportPath = '';

  final items = signal<List<OfflineItem>>(const []);

  /// The folder new downloads go to; null is the app's own storage.
  final targetDirectory = signal<String?>(null);

  /// Bytes taken by finished files.
  late final usedBytes = computed(
    () => items.value
        .where((i) => i.isFinished)
        .fold<int>(0, (sum, i) => sum + i.sizeBytes),
  );

  GraphqlService get _graphql => getIt<GraphqlService>();
  SessionService get _session => getIt<SessionService>();

  bool get _wifiOnly => _session.device.value?.downloadsWifiOnly ?? true;

  Future<void> load() async {
    _supportPath = (await getApplicationSupportDirectory()).path;
    final prefs = await SharedPreferences.getInstance();
    targetDirectory.value = prefs.getString(_targetKey);
    await _configure();
    _updates = FileDownloader().updates.listen(_onUpdate);
    await FileDownloader().start();
    // Reconcile once the device has registered, on getting back on a network,
    // and now and then while the app is open.
    _stopWatching = effect(() {
      final device = _session.device.value;
      final online = getIt<NetworkService>().online.value;
      final unmetered = getIt<NetworkService>().unmetered.value;
      if (device != null && online) {
        untracked(() {
          if (unmetered || !device.downloadsWifiOnly) unawaited(reconcile());
        });
      }
    });
    _lifecycle = AppLifecycleListener(onResume: _onResume, onPause: _onPause);
    await _startForegroundWork();
  }

  /// Whose downloads these are from now on, or nobody's with null. The session
  /// says so as someone is chosen or leaves. Transfers of whoever was here carry
  /// on in the downloader, and their manifest catches up when they're back.
  Future<void> useAccount(String? accountId) async {
    if (accountId == _store.accountId) return;
    _saveDebounce?.cancel();
    await _store.save();
    final store = OfflineStore(accountId);
    await store.load();
    _store = store;
    items.value = store.items;
    await _release();
    await _syncWithDownloader();
  }

  /// Stop the active account's transfers before its session is locked. Their
  /// URLs carry tokens the lock refuses, so left running they'd fail; stopped,
  /// they pick up from [_release] with the same URLs, which the PIN makes good
  /// again. What the viewer paused themselves is left paused.
  Future<void> hold() async {
    final held = <String>[];
    for (final item in _store.items) {
      if (item.isFinished ||
          item.status == OfflineStatus.failed ||
          item.status == OfflineStatus.paused) {
        continue;
      }
      final task = await FileDownloader().taskForId(item.taskId);
      if (task is! DownloadTask) continue;
      // Only a running transfer can pause; one still queued is taken out, and
      // goes back in from its record.
      if (!await FileDownloader().pause(task)) {
        await FileDownloader().cancelTaskWithId(item.taskId);
      }
      held.add(item.taskId);
    }
    if (held.isEmpty) return;
    _store.held = held;
    _saveDebounce?.cancel();
    await _store.save();
  }

  Future<void> _release() async {
    final held = _store.held;
    if (held.isEmpty) return;
    _store.held = const [];
    for (final taskId in held) {
      final record = await FileDownloader().database.recordForId(taskId);
      final task = record?.task;
      if (task is! DownloadTask) continue;
      if (record!.status == TaskStatus.paused) {
        await FileDownloader().resume(task);
      } else {
        await FileDownloader().enqueue(task);
      }
    }
    _saveNow();
  }

  /// The headless sync stands down for what's marked here, and this stops when
  /// the app leaves the screen, so it never has both copies working at once.
  Future<void> _startForegroundWork() async {
    _periodic?.cancel();
    _stamp?.cancel();
    _periodic = Timer.periodic(
      const Duration(minutes: 30),
      (_) => unawaited(reconcile()),
    );
    await _markForeground();
    _stamp = Timer.periodic(
      const Duration(minutes: 1),
      (_) => unawaited(_markForeground()),
    );
  }

  void _onPause() {
    _periodic?.cancel();
    _stamp?.cancel();
    _saveNow();
    unawaited(_clearForeground());
  }

  /// The headless sync may have changed the manifest and rotated the session
  /// while this isolate was frozen, and transfers carried on without it.
  Future<void> _onResume() async {
    await _startForegroundWork();
    await getIt<TokenStore>().reloadTokens();
    // The session it rotated may also be one it found ended.
    if (await _session.lostWhileAway()) return;
    await _store.load();
    items.value = _store.items;
    await _syncWithDownloader();
    await reconcile();
  }

  static Future<void> _clearForeground() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove(foregroundStampKey);
  }

  static Future<void> _markForeground() async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setInt(
      foregroundStampKey,
      DateTime.now().millisecondsSinceEpoch,
    );
  }

  Future<void> _configure() async {
    await FileDownloader().configure(
      globalConfig: [
        (Config.requestTimeout, const Duration(seconds: 60)),
        // On iOS a resource timeout ends a task, however it's doing. A film
        // over a slow link, after a wait for Wi-Fi, takes far longer than the
        // default 4 hours. It can only be set before the first task runs.
        (Config.resourceTimeout, const Duration(days: 7)),
        // Leave room for the system; a full disk breaks more than downloads.
        (Config.checkAvailableSpace, 1024),
        (Config.excludeFromCloudBackup, Config.always),
        // Android's cache quota is too small for films, and it can vanish.
        (Config.useCacheDir, Config.never),
      ],
    );
    FileDownloader().configureNotificationForGroup(
      _mediaGroup,
      running: const TaskNotification('Downloading', '{displayName}'),
      complete: const TaskNotification(
        'Ready to watch offline',
        '{displayName}',
      ),
      error: const TaskNotification('Download stopped', '{displayName}'),
      progressBar: true,
    );
  }

  /// Transfers carry on without the app, so the manifest can be behind.
  Future<void> _syncWithDownloader() async {
    final records = await FileDownloader().database.allRecords(
      group: _mediaGroup,
    );
    final byId = {for (final record in records) record.taskId: record};
    var changed = false;
    final next = [
      for (final item in _store.items)
        () {
          final record = byId[item.taskId];
          if (record == null || item.isFinished) return item;
          final status = _statusFrom(record.status);
          if (status == item.status && record.progress == item.progress) {
            return item;
          }
          changed = true;
          return item.copyWith(
            status: status,
            progress: record.progress < 0 ? item.progress : record.progress,
          );
        }(),
    ];
    if (changed) _setItems(next);
    for (final item in next) {
      if (item.isFinished) await _recordSize(item.taskId);
    }
  }

  /// An export's size isn't known until it's here, and what's kept on the
  /// device is counted from these.
  Future<void> _recordSize(String taskId) async {
    final item = _store.items.where((i) => i.taskId == taskId).firstOrNull;
    if (item == null || item.sizeBytes > 0) return;
    final path = await localPathOf(item);
    if (path == null) return;
    final size = await File(path).length();
    _update(taskId, (item) => item.copyWith(sizeBytes: size));
  }

  static OfflineStatus _statusFrom(TaskStatus status) => switch (status) {
    TaskStatus.running => OfflineStatus.downloading,
    TaskStatus.complete => OfflineStatus.complete,
    TaskStatus.paused => OfflineStatus.paused,
    TaskStatus.notFound ||
    TaskStatus.failed ||
    TaskStatus.canceled => OfflineStatus.failed,
    TaskStatus.enqueued || TaskStatus.waitingToRetry => OfflineStatus.queued,
  };

  void _onUpdate(TaskUpdate update) {
    if (update.task.group != _mediaGroup) return;
    final id = update.task.taskId;
    switch (update) {
      case TaskStatusUpdate():
        final status = _statusFrom(update.status);
        _update(
          id,
          (item) => item.copyWith(
            status: status,
            progress: status == OfflineStatus.complete ? 1 : item.progress,
            error: status == OfflineStatus.failed
                ? update.exception?.description ?? 'The download stopped.'
                : null,
            clearError: status != OfflineStatus.failed,
          ),
        );
        if (status == OfflineStatus.complete) {
          _saveNow();
          unawaited(_recordSize(id));
        }
      case TaskProgressUpdate():
        if (update.progress >= 0) {
          _update(
            id,
            (item) => item.copyWith(
              progress: update.progress,
              status: OfflineStatus.downloading,
            ),
          );
        }
    }
  }

  void _update(String taskId, OfflineItem Function(OfflineItem) change) {
    // Someone else's transfer: theirs to hear about when they're active.
    if (!_store.items.any((item) => item.taskId == taskId)) return;
    _setItems([
      for (final item in _store.items)
        item.taskId == taskId ? change(item) : item,
    ]);
  }

  void _setItems(List<OfflineItem> next) {
    _store.items = next;
    items.value = next;
    _saveDebounce?.cancel();
    _saveDebounce = Timer(const Duration(seconds: 2), _saveNow);
  }

  void _saveNow() {
    _saveDebounce?.cancel();
    unawaited(_store.save());
  }

  /// A finished copy of the first part, which is what playing a title opens.
  OfflineItem? completedFor(String workId) {
    for (final item in _store.items) {
      if (item.work.id == workId && item.isFinished && item.partIndex == 0) {
        return item;
      }
    }
    return null;
  }

  /// Every file for a work, reactively, for a button to reflect.
  List<OfflineItem> itemsFor(String workId) => [
    for (final item in items.value)
      if (item.work.id == workId) item,
  ];

  Future<String?> localPathOf(OfflineItem item) => mediaPathOf(item);

  Future<String?> localSubtitlePath(
    OfflineItem item,
    OfflineSubtitle subtitle,
  ) async {
    final path = p.join(
      _supportPath,
      offlineDirectory(item.accountId),
      subtitle.fileName,
    );
    return File(path).existsSync() ? path : null;
  }

  String? localPosterPath(OfflineItem item) {
    final file = item.posterFile;
    if (file == null) return null;
    final path = p.join(_supportPath, offlineDirectory(item.accountId), file);
    return File(path).existsSync() ? path : null;
  }

  /// Keep one version of a title on this device. Returns false when the server
  /// has nothing this device can play yet; an export can stand in. [server] is
  /// the linked server the title is on, which needs to know this device too.
  Future<bool> download(
    MediaVersion version, {
    WorkSummary? seed,
    ServerRef? server,
  }) async {
    final identity = getIt<DeviceIdentity>();
    if (server != null) await _registerOn(server);
    final data = await _graphql.run(
      GDeviceDownloadReq(
        (b) => b
          ..fetchPolicy = FetchPolicy.NetworkOnly
          ..vars.device = identity.id
          ..vars.media = version.id,
      ),
      server: server,
    );
    final answer = data.deviceDownload;
    if (answer == null) return false;
    final file = wantedFromDownload(answer, server: server);
    unawaited(_ensurePermission());
    await _enqueue(file, OfflineOrigin.manual);
    return true;
  }

  Future<void> _registerOn(ServerRef server) async {
    final identity = getIt<DeviceIdentity>();
    final profile = await getIt<DecoderProbeService>().ready();
    await _graphql.run(
      GRegisterDeviceReq(
        (b) => b.vars
          ..id = identity.id
          ..name = identity.name
          ..capabilities.replace(buildCapabilities(profile)),
      ),
      server: server,
    );
  }

  ServerRef? _linked(String? id) => id == null
      ? null
      : getIt<SessionService>().connectedServers
            .peek()
            .where((server) => server.id == id)
            .firstOrNull;

  /// [item]'s work, with its linked server if the session has that server.
  WorkSummary workOf(OfflineItem item) => item.serverId == null
      ? item.work
      : item.work.withServer(_linked(item.serverId));

  Future<void> _ensurePermission() async {
    final permissions = FileDownloader().permissions;
    final status = await permissions.status(PermissionType.notifications);
    if (status != PermissionStatus.granted) {
      await permissions.request(PermissionType.notifications);
    }
  }

  Future<void> _enqueue(WantedFile file, OfflineOrigin origin) async {
    final tokens = getIt<TokenStore>();
    final account = tokens.account(_store.accountId);
    if (account == null) return;
    final existing = _store.items
        .where((item) => item.key == file.key)
        .firstOrNull;
    if (existing != null && existing.status != OfflineStatus.failed) {
      // Asking to keep what the server fetched makes it the viewer's to keep.
      if (origin == OfflineOrigin.manual &&
          existing.origin == OfflineOrigin.smart) {
        _update(existing.taskId, (item) => item.copyWith(origin: origin));
      }
      return;
    }
    final item = itemFor(
      file,
      origin,
      accountId: account.userId,
      directory: chosenDirectory(targetDirectory.value, account),
    );
    _setItems([
      for (final other in _store.items)
        if (other.key != item.key) other,
      item,
    ]);
    final server = file.work.server;
    String absolute(String url) => server != null && url.startsWith('/')
        ? '${server.origin}$url'
        : tokens.absolute(url);
    await enqueueTransfer(item, absolute(file.url), wifiOnly: _wifiOnly);
    await enqueueExtras(item, file, absolute: absolute);
  }

  /// Stop a transfer and delete what's on disk.
  Future<void> remove(OfflineItem item) async {
    await FileDownloader().cancelTaskWithId(item.taskId);
    await deleteOfflineFiles(item);
    await FileDownloader().database.deleteRecordWithId(item.taskId);
    _setItems([
      for (final other in _store.items)
        if (other.key != item.key) other,
    ]);
    _saveNow();
  }

  /// Everything of the active account's, as they sign out.
  Future<void> removeAll() async {
    final store = _store;
    for (final item in [...store.items]) {
      await remove(item);
    }
    _saveDebounce?.cancel();
    _store = OfflineStore(null);
    items.value = const [];
    await store.delete();
  }

  /// Everyone's, signed in or not, as the device leaves the server.
  Future<void> removeEveryone() async {
    await removeAll();
    for (final accountId in await OfflineStore.accountIds()) {
      final store = OfflineStore(accountId);
      await store.load();
      for (final item in store.items) {
        await FileDownloader().cancelTaskWithId(item.taskId);
        await deleteOfflineFiles(item);
        await FileDownloader().database.deleteRecordWithId(item.taskId);
      }
      await store.delete();
    }
  }

  Future<void> pause(OfflineItem item) async {
    final task = await FileDownloader().taskForId(item.taskId);
    if (task is DownloadTask) await FileDownloader().pause(task);
  }

  Future<void> resume(OfflineItem item) async {
    final record = await FileDownloader().database.recordForId(item.taskId);
    final task = record?.task;
    if (task is DownloadTask) await FileDownloader().resume(task);
  }

  /// Ask again for a failed title; the old URL may be what went wrong.
  Future<void> retry(OfflineItem item, MediaVersion? version) async {
    final identity = getIt<DeviceIdentity>();
    await deleteOfflineFiles(item);
    if (version != null) {
      await download(version);
      return;
    }
    // A smart title comes back with the next reconcile.
    _setItems([
      for (final other in _store.items)
        if (other.key != item.key) other,
    ]);
    if (identity.id.isNotEmpty) await reconcile();
  }

  /// Compare with what the server wants here, then fetch and evict.
  Future<void> reconcile() async {
    if (_reconciling || _session.phase.value != AppPhase.app) return;
    final device = _session.device.value;
    if (device == null) return;
    final store = _store;
    _reconciling = true;
    try {
      await flushReports();
      if (!device.smartDownloads) {
        await _apply(
          planSmartDownloads(
            enabled: false,
            wanted: const [],
            items: _store.items,
          ),
        );
        return;
      }
      final data = await _graphql.run(
        GSmartDownloadsReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NetworkOnly
            ..vars.device = device.id,
        ),
      );
      // Someone else is active by now, and the answer was for whoever asked.
      if (!identical(store, _store)) return;
      final plan = planSmartDownloads(
        enabled: true,
        wanted: [
          for (final download in data.smartDownloads)
            wantedFromDownload(download),
        ],
        items: _store.items,
      );
      await _apply(plan);
    } on Exception catch (error) {
      // Next time; downloads aren't worth interrupting anyone over.
      debugPrint('smart downloads: $error');
    } finally {
      _reconciling = false;
    }
  }

  Future<void> _apply(SmartPlan plan) async {
    final store = _store;
    for (final item in plan.evict) {
      if (!identical(store, _store)) return;
      await remove(item);
    }
    for (final file in plan.fetch) {
      if (!identical(store, _store)) return;
      await _enqueue(file, OfflineOrigin.smart);
    }
  }

  // ---- offline progress -------------------------------------------------

  void recordProgress(
    String workId, {
    required int positionMs,
    bool played = false,
  }) {
    if (!_store.items.any((item) => item.work.id == workId)) return;
    _setItems([
      for (final item in _store.items)
        item.work.id == workId
            ? item.copyWith(
                positionMs: positionMs,
                played: played || item.played,
              )
            : item,
    ]);
  }

  /// The server was away; tell it later. Only the latest position matters.
  void queueProgress(
    String workId, {
    required int positionMs,
    bool played = false,
    ServerRef? server,
  }) {
    _store.reports = [
      for (final report in _store.reports)
        if (report.workId != workId) report,
      PendingReport(
        workId: workId,
        positionMs: positionMs,
        played: played,
        serverId: server?.id,
      ),
    ];
    _saveDebounce?.cancel();
    _saveDebounce = Timer(const Duration(seconds: 2), _saveNow);
  }

  Future<void> flushReports() async {
    final pending = _store.reports;
    if (pending.isEmpty) return;
    final remaining = <PendingReport>[];
    for (final report in pending) {
      final server = _linked(report.serverId);
      // Its server isn't back yet, as far as this session knows.
      if (report.serverId != null && server == null) {
        remaining.add(report);
        continue;
      }
      try {
        if (report.played) {
          await _graphql.run(
            GMarkPlayedReq((b) => b.vars.id = report.workId),
            server: server,
          );
        } else {
          await _graphql.run(
            GReportProgressReq(
              (b) => b.vars
                ..id = report.workId
                ..position = report.positionMs
                ..state = GPlaybackState.STOPPED,
            ),
            server: server,
          );
        }
      } on ServerUnreachable {
        remaining.add(report);
      } on Exception {
        // Refused: the title is gone or no longer theirs, so stop asking.
      }
    }
    _store.reports = remaining;
    _saveNow();
  }

  // ---- settings ---------------------------------------------------------

  Future<void> setTargetDirectory(String? path) async {
    targetDirectory.value = path;
    final prefs = await SharedPreferences.getInstance();
    if (path == null) {
      await prefs.remove(_targetKey);
    } else {
      await prefs.setString(_targetKey, path);
    }
  }

  /// Change a server-held setting of this device.
  Future<void> updateDevice({
    bool? smartDownloads,
    bool? wifiOnly,
    int? budgetBytes,
    bool clearBudget = false,
    bool? smartTranscode,
    int? maxBitrateBps,
    bool clearBitrate = false,
  }) async {
    final identity = getIt<DeviceIdentity>();
    // Asked now, with the viewer here, as a reconcile may run with nobody looking.
    if (smartDownloads == true) unawaited(_ensurePermission());
    final result = await getIt<ToastService>().attempt(
      () => _graphql.run(
        GUpdateDeviceReq(
          (b) => b.vars
            ..id = identity.id
            ..smartDownloads = smartDownloads
            ..downloadsWifiOnly = wifiOnly
            ..downloadBudgetBytes = budgetBytes
            ..clearDownloadBudget = clearBudget ? true : null
            ..smartTranscode = smartTranscode
            ..maxBitrateBps = maxBitrateBps
            ..clearMaxBitrate = clearBitrate ? true : null,
        ),
      ),
    );
    if (result == null) return;
    _session.device.value = result.updateDevice;
    if (wifiOnly != null) {
      await FileDownloader().requireWiFi(
        wifiOnly ? RequireWiFi.forAllTasks : RequireWiFi.forNoTasks,
        rescheduleRunningTasks: true,
      );
    }
    unawaited(reconcile());
  }

  /// A copy packaged on the server, for a title with nothing ready for this
  /// device. It's a stream copy, so it changes the container and not the codecs.
  Future<void> createExport(
    String editionId, {
    ServerRef? server,
    GExportFormat? format,
  }) async {
    await getIt<ToastService>().attempt(
      () => _graphql.run(
        GCreateExportJobReq(
          (b) => b.vars
            ..edition = editionId
            ..format = format ?? GExportFormat.MP4,
        ),
        server: server,
      ),
      success: 'Making an offline copy. It’ll be under You → Offline copies when ready.',
    );
  }

  /// Bring a finished export onto this device. Unlike the token-bearing routes
  /// this one wants a header, so on the home server the access token has to be
  /// fresh now. A linked server's key doesn't expire.
  Future<void> fetchExport({
    required String jobId,
    required WorkSummary work,
  }) async {
    final tokens = getIt<TokenStore>();
    final account = tokens.account(_store.accountId);
    if (account == null) return;
    final server = work.server;
    if (server == null) await tokens.refresh();
    unawaited(_ensurePermission());
    final item = itemFor(
      WantedFile(work: work, partIndex: 0, url: '', sizeBytes: 0),
      OfflineOrigin.manual,
      accountId: account.userId,
      directory: chosenDirectory(targetDirectory.value, account),
    );
    _setItems([
      for (final other in _store.items)
        if (other.key != item.key) other,
      item,
    ]);
    await FileDownloader().enqueue(
      DownloadTask(
        taskId: item.taskId,
        group: _mediaGroup,
        url: server != null
            ? '${server.origin}/download/$jobId/file'
            : tokens.absolute('/download/$jobId/file'),
        headers: server != null
            ? {'Authorization': 'Bearer ${server.apiKey}'}
            : tokens.authHeaders,
        filename: item.fileName,
        directory: item.directory ?? offlineDirectory(item.accountId),
        baseDirectory: item.directory != null
            ? BaseDirectory.root
            : BaseDirectory.applicationSupport,
        updates: Updates.statusAndProgress,
        requiresWiFi: _wifiOnly,
        allowPause: true,
        retries: 5,
        displayName: work.heading,
      ),
    );
  }

  void dispose() {
    unawaited(_updates?.cancel());
    _periodic?.cancel();
    _stamp?.cancel();
    _saveDebounce?.cancel();
    _stopWatching?.call();
    _lifecycle?.dispose();
  }
}
