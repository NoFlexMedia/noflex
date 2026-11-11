// SPDX-License-Identifier: AGPL-3.0-only
//
// Flutter's license registry knows every Dart package, but not the native
// libraries media_kit downloads prebuilt for each platform, nor the bundled font.
// assets/licenses/ has them, written by `cargo xtask third-party sync` from
// packaging/third-party/apps.toml: change that, never these.

import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

String? get _platform => switch (defaultTargetPlatform) {
  TargetPlatform.android => 'android',
  TargetPlatform.iOS => 'ios',
  TargetPlatform.macOS => 'macos',
  TargetPlatform.windows => 'windows',
  TargetPlatform.linux => 'linux',
  TargetPlatform.fuchsia => null,
};

void registerNativeLicenses() => LicenseRegistry.addLicense(_entries);

Stream<LicenseEntry> _entries() async* {
  Future<String> load(String name) =>
      rootBundle.loadString('assets/licenses/$name');
  final manifest = jsonDecode(await load('manifest.json')) as Map;
  final libraries = (manifest['platforms'] as Map)[_platform] as List? ?? [];
  for (final library in libraries.cast<Map>()) {
    final parts = [
      if (library['version'] case final String version) 'Version $version.',
      for (final file in (library['files'] as List).cast<String>())
        await load(file),
    ];
    yield LicenseEntryWithLineBreaks([
      library['name'] as String,
    ], parts.join('\n\n'));
  }
}

/// Everything this build carries, by what it's called, in alphabetical order:
/// Dart packages, Flutter's engine, and the libraries above.
Future<List<(String, List<LicenseEntry>)>> licensesByPackage() async {
  final byPackage = <String, List<LicenseEntry>>{};
  await for (final entry in LicenseRegistry.licenses) {
    for (final package in entry.packages) {
      (byPackage[package] ??= []).add(entry);
    }
  }
  return [for (final MapEntry(:key, :value) in byPackage.entries) (key, value)]
    ..sort((a, b) => a.$1.toLowerCase().compareTo(b.$1.toLowerCase()));
}
