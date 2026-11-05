// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:media_kit/media_kit.dart';
import 'package:window_manager/window_manager.dart';

import 'app.dart';
import 'di.dart';
import 'features/downloads/background_sync.dart';
import 'features/downloads/downloads_service.dart';
import 'features/player/decoder_probe.dart';
import 'services/device_identity.dart';
import 'services/session.dart';
import 'services/tokens.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  MediaKit.ensureInitialized();
  if (Platform.isLinux || Platform.isMacOS || Platform.isWindows) {
    await windowManager.ensureInitialized();
    await windowManager.setMinimumSize(const Size(360, 480));
  }
  configureDependencies();
  await getIt<TokenStore>().load();
  await getIt<DeviceIdentity>().load();
  await getIt<DownloadsService>().load();
  runApp(const NoFlexApp());
  unawaited(getIt<SessionService>().start());
  unawaited(scheduleSmartSync());
  // Measured while the user browses, so the first play doesn't wait on it.
  unawaited(getIt<DecoderProbeService>().ready());
}
