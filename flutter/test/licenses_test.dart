// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:noflex/services/licenses.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
    LicenseRegistry.reset();
  });

  // `cargo xtask third-party check` holds the assets to their record; this holds
  // the app to the manifest's shape, on every platform.
  test('every platform’s native libraries come with their licenses', () async {
    for (final platform in [
      TargetPlatform.android,
      TargetPlatform.iOS,
      TargetPlatform.macOS,
      TargetPlatform.windows,
      TargetPlatform.linux,
    ]) {
      debugDefaultTargetPlatformOverride = platform;
      LicenseRegistry.reset();
      registerNativeLicenses();
      final entries = await LicenseRegistry.licenses.toList();
      expect(entries, isNotEmpty, reason: '$platform');
      for (final entry in entries) {
        final text = entry.paragraphs.map((p) => p.text).join('\n');
        // The LGPL asks for its own text beside each library under it.
        if (text.contains('Lesser General Public License')) {
          expect(
            text,
            contains('GNU LESSER GENERAL PUBLIC LICENSE'),
            reason: '$platform: ${entry.packages.join()}',
          );
        }
      }
    }
  });
}
