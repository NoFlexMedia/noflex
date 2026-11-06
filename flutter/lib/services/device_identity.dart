// SPDX-License-Identifier: AGPL-3.0-only
//
// This install as a registered device. The id isn't a credential; it has to
// outlive sign-ins for the device, and what the server prepares for it, to stay
// the same.

import 'dart:io';

import 'package:device_info_plus/device_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';

const _deviceIdKey = 'noflex.deviceId';

class DeviceIdentity {
  late final String id;
  late final String name;

  /// What sessions are listed under in Account → Devices.
  String get clientName => 'NoFlex for $platformName';

  static String get platformName => Platform.isAndroid
      ? 'Android'
      : Platform.isIOS
      ? 'iOS'
      : Platform.isMacOS
      ? 'macOS'
      : Platform.isWindows
      ? 'Windows'
      : 'Linux';

  Future<void> load() async {
    final prefs = await SharedPreferences.getInstance();
    final existing = prefs.getString(_deviceIdKey);
    id = existing ?? const Uuid().v4();
    if (existing == null) await prefs.setString(_deviceIdKey, id);
    name = await _deviceName();
  }

  static Future<String> _deviceName() async {
    final info = DeviceInfoPlugin();
    try {
      if (Platform.isAndroid) {
        final android = await info.androidInfo;
        // A model alone is often a code like SM-S918B.
        return android.model.toLowerCase().startsWith(
              android.manufacturer.toLowerCase(),
            )
            ? android.model
            : '${android.manufacturer} ${android.model}';
      }
      if (Platform.isIOS) return (await info.iosInfo).name;
      if (Platform.isMacOS) return (await info.macOsInfo).computerName;
      if (Platform.isWindows) return (await info.windowsInfo).computerName;
      if (Platform.isLinux) return (await info.linuxInfo).prettyName;
    } catch (_) {
      // A name is a courtesy; the platform stands in.
    }
    return platformName;
  }
}
