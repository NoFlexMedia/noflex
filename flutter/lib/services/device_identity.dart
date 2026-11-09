// SPDX-License-Identifier: AGPL-3.0-only
//
// This install as a registered device. The id isn't a credential; it has to
// outlive sign-ins for the device, and what the server prepares for it, to stay
// the same.
//
// A device belongs to one user on the server, which refuses an id someone else
// registered. So each person signed in here has an id of their own: to the
// server this is their device, with their settings and what's prepared for them.

import 'dart:async';
import 'dart:io';

import 'package:device_info_plus/device_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';

import 'tokens.dart';

/// Where [userId]'s id for this device is kept. Not with their session, which
/// signing out forgets.
String deviceIdKey(String userId) => 'noflex.deviceId.$userId';

class DeviceIdentity {
  DeviceIdentity(this._tokens);

  final TokenStore _tokens;
  late final SharedPreferences _prefs;
  late final String name;

  /// The active account's id for this device; empty with nobody active.
  String get id {
    final userId = _tokens.activeId.value;
    return userId == null ? '' : idFor(userId);
  }

  String idFor(String userId) {
    final key = deviceIdKey(userId);
    final existing = _prefs.getString(key);
    if (existing != null) return existing;
    final id = const Uuid().v4();
    // Readable at once; the write to disk follows.
    unawaited(_prefs.setString(key, id));
    return id;
  }

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
    _prefs = await SharedPreferences.getInstance();
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
