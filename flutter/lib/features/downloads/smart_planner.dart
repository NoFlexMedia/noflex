// SPDX-License-Identifier: AGPL-3.0-only
//
// What a reconcile should do, given what the server wants on this device and
// what's already here. The server ranks by what's likely to be watched and stays
// within the budget, so this only compares; pure, so the rules can be tested.

import 'download_models.dart';

class SmartPlan {
  const SmartPlan({required this.fetch, required this.evict});

  /// Most important first.
  final List<WantedFile> fetch;
  final List<OfflineItem> evict;

  bool get isEmpty => fetch.isEmpty && evict.isEmpty;
}

/// A title the server no longer wants is removed if the server put it here: it's
/// been watched, or something likelier has displaced it. What the viewer chose
/// to keep is never touched, and counts toward nothing the server decides.
///
/// With smart downloads off, nothing new starts and transfers underway stop, but
/// finished files stay; turning a setting off shouldn't delete someone's films.
SmartPlan planSmartDownloads({
  required bool enabled,
  required List<WantedFile> wanted,
  required List<OfflineItem> items,
}) {
  if (!enabled) {
    return SmartPlan(
      fetch: const [],
      evict: [
        for (final item in items)
          if (item.origin == OfflineOrigin.smart && !item.isFinished) item,
      ],
    );
  }
  final wantedKeys = {for (final file in wanted) file.key};
  final byKey = {for (final item in items) item.key: item};
  return SmartPlan(
    fetch: [
      for (final file in wanted)
        // A failed transfer goes round again with the fresh URL this just got.
        if (byKey[file.key] == null ||
            byKey[file.key]!.status == OfflineStatus.failed)
          file,
    ],
    evict: [
      for (final item in items)
        if (item.origin == OfflineOrigin.smart &&
            !wantedKeys.contains(item.key))
          item,
    ],
  );
}
