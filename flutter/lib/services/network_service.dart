// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:connectivity_plus/connectivity_plus.dart';
import 'package:signals_flutter/signals_flutter.dart';

/// Connectivity, mirrored into signals once, here.
class NetworkService {
  NetworkService() {
    final connectivity = Connectivity();
    unawaited(connectivity.checkConnectivity().then(_apply));
    _subscription = connectivity.onConnectivityChanged.listen(_apply);
  }

  late final StreamSubscription<List<ConnectivityResult>> _subscription;

  final online = signal(true);

  /// Wi-Fi or a wire, the connections downloads may wait for.
  final unmetered = signal(true);

  void _apply(List<ConnectivityResult> results) {
    batch(() {
      online.value = results.any((r) => r != ConnectivityResult.none);
      unmetered.value = results.any(
        (r) => r == ConnectivityResult.wifi || r == ConnectivityResult.ethernet,
      );
    });
  }

  void dispose() => unawaited(_subscription.cancel());
}
