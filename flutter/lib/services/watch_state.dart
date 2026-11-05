// SPDX-License-Identifier: AGPL-3.0-only
//
// Play state as it changes on this device. Lists keep the answer they were
// fetched with; a card reads through here, so progress and favorites update
// everywhere at once, offline included, without refetching or cache surgery.

import 'package:signals_flutter/signals_flutter.dart';

import '../models/work.dart';

class WatchStateService {
  final _overrides = signal<Map<String, WatchData>>(const {});

  /// Reads the override reactively, so call this inside a SignalBuilder.
  WatchData of(WorkSummary work) =>
      _overrides.value[work.id] ?? work.userData ?? const WatchData();

  WatchData? overrideFor(String workId) => _overrides.value[workId];

  void set(String workId, WatchData data) =>
      _overrides.value = {..._overrides.value, workId: data};

  void clear() => _overrides.value = const {};
}
