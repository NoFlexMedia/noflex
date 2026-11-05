// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:signals_flutter/signals_flutter.dart';

/// Android's picture-in-picture, bridged once, here. libmpv draws into a Flutter
/// texture rather than an AVPlayer layer, so iOS's has nothing to show, and
/// desktops have windows.
class PipService {
  PipService() {
    if (supported) {
      _channel.setMethodCallHandler((call) async {
        if (call.method == 'changed') active.value = call.arguments as bool;
      });
    }
  }

  static const _channel = MethodChannel('live.noflex/pip');

  bool get supported => Platform.isAndroid;

  /// In the small window, where the player shows the picture and nothing else.
  final active = signal(false);

  /// Leaving the app goes to picture-in-picture only while something plays.
  void setPlaying(bool playing, {int? width, int? height}) {
    if (!supported) return;
    unawaited(
      _channel.invokeMethod<void>('setPlaying', {
        'playing': playing,
        'width': width,
        'height': height,
      }),
    );
  }

  void enter() {
    if (supported) unawaited(_channel.invokeMethod<void>('enter'));
  }
}
