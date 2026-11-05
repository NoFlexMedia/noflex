// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:signals_flutter/signals_flutter.dart';

import 'tokens.dart';

enum ToastKind { info, success, error }

class Toast {
  const Toast(
    this.id,
    this.message,
    this.kind, {
    this.actionLabel,
    this.action,
  });
  final int id;
  final String message;
  final ToastKind kind;
  final String? actionLabel;
  final void Function()? action;
}

class ToastService {
  final toasts = signal<List<Toast>>(const []);
  int _next = 0;

  void show(
    String message, {
    ToastKind kind = ToastKind.info,
    String? actionLabel,
    void Function()? action,
  }) {
    final toast = Toast(
      _next++,
      message,
      kind,
      actionLabel: actionLabel,
      action: action,
    );
    toasts.value = [...toasts.value, toast];
    Timer(
      Duration(seconds: kind == ToastKind.error ? 6 : 4),
      () => dismiss(toast.id),
    );
  }

  void dismiss(int id) => toasts.value = [
    for (final toast in toasts.value)
      if (toast.id != id) toast,
  ];

  /// Run a user-triggered action: failures surface as a toast, success can
  /// confirm with one. Returns the result, or null when it failed.
  Future<T?> attempt<T>(Future<T> Function() action, {String? success}) async {
    try {
      final result = await action();
      if (success != null) show(success, kind: ToastKind.success);
      return result;
    } on SessionExpired {
      return null;
    } on Exception catch (error) {
      show(error.toString(), kind: ToastKind.error);
      return null;
    }
  }
}
