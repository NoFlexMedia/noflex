// SPDX-License-Identifier: AGPL-3.0-only
//
// The one seam between Ferry's streams and the app: a request mirrored into
// signals. Screens and services hold these and read them inside SignalBuilder;
// nothing else in the app listens to a GraphQL stream.

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../services/graphql.dart';

class Query<TData, TVars> {
  Query(this._client, this._request) {
    _listen();
  }

  final Client _client;
  final OperationRequest<TData, TVars> _request;
  StreamSubscription<OperationResponse<TData, TVars>>? _subscription;

  /// Last good answer; kept through a failed refetch.
  final data = signal<TData?>(null);
  final error = signal<Exception?>(null);
  final loading = signal(true);

  /// Nothing to show yet.
  late final waiting = computed(() => data.value == null && loading.value);

  void _listen() {
    _subscription = _client.request(_request).listen((response) {
      batch(() {
        if (response.data != null) data.value = response.data;
        error.value = failureOf(response);
        // Ferry calls a response with only GraphQL errors still loading.
        loading.value = response.loading && !response.hasErrors;
      });
    });
  }

  /// Ask again. The cache answers first when it can, then the network.
  void refetch() {
    loading.value = true;
    unawaited(_subscription?.cancel());
    _listen();
  }

  void dispose() {
    unawaited(_subscription?.cancel());
  }
}

/// A cursor-paged list. Pages arrive through [fetch], which returns the items
/// and the next cursor (null at the end).
class Pager<T> {
  Pager(this.fetch);

  final Future<(List<T>, String?)> Function(String? after) fetch;

  final items = signal<List<T>>(const []);
  final loading = signal(false);
  final error = signal<Exception?>(null);
  final hasMore = signal(true);
  String? _cursor;
  int _generation = 0;

  late final waiting = computed(() => items.value.isEmpty && loading.value);

  Future<void> refresh() {
    _generation++;
    _cursor = null;
    batch(() {
      hasMore.value = true;
      loading.value = false;
    });
    return _load(replace: true);
  }

  Future<void> loadMore() => _load(replace: false);

  Future<void> _load({required bool replace}) async {
    if (loading.value || !hasMore.value) return;
    final generation = _generation;
    batch(() {
      loading.value = true;
      error.value = null;
    });
    try {
      final (page, next) = await fetch(_cursor);
      if (generation != _generation) return;
      _cursor = next;
      batch(() {
        items.value = replace ? page : [...items.value, ...page];
        hasMore.value = next != null;
      });
    } on Exception catch (cause) {
      if (generation == _generation) error.value = cause;
    } finally {
      if (generation == _generation) loading.value = false;
    }
  }
}
