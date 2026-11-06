// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:simple_icons/simple_icons.dart';
import 'package:noflex/utils/device_icon.dart';
import 'package:noflex/utils/picture.dart';
import 'package:noflex/widgets/user_avatar.dart';

Future<Uint8List> _png(int width, int height) async {
  final recorder = ui.PictureRecorder();
  ui.Canvas(recorder).drawColor(const ui.Color(0xFF3366CC), ui.BlendMode.src);
  final image = await recorder.endRecording().toImage(width, height);
  final data = await image.toByteData(format: ui.ImageByteFormat.png);
  return data!.buffer.asUint8List();
}

Future<(int, int)> _size(String encoded) async {
  final codec = await ui.instantiateImageCodec(base64Decode(encoded));
  final image = (await codec.getNextFrame()).image;
  return (image.width, image.height);
}

void main() {
  test('initials are the first and last words, or a lone word’s start', () {
    expect(initialsOf('Vera Viewer'), 'VV');
    expect(initialsOf('Vera van der Viewer'), 'VV');
    expect(initialsOf('vera'), 'VE');
    expect(initialsOf('  '), '');
    expect(initialsOf(null), '');
  });

  // lib/ts/device.ts holds the same table, in solidtv/src/lib/device.test.ts.
  test('a device is read from what it reports', () {
    for (final (device, client, icon) in [
      ('Google TV (Chromecast HD)', 'NoFlex TV', SimpleIcons.googletv),
      ('Android TV (SHIELD Android TV)', 'NoFlex TV', SimpleIcons.nvidia),
      ('Fire TV (AFTMM)', 'NoFlex TV', PhosphorIconsRegular.television),
      ('Samsung TV', 'NoFlex TV', PhosphorIconsRegular.television),
      ('Apple TV', 'NoFlex TV', PhosphorIconsFill.appleLogo),
      ('Brave on Linux', 'NoFlex Web', SimpleIcons.brave),
      ('Firefox on Android', 'NoFlex Web', SimpleIcons.firefoxbrowser),
      ('Chrome on iPhone', 'NoFlex Web', SimpleIcons.googlechrome),
      ('Edge on Windows', 'NoFlex Web', PhosphorIconsRegular.browser),
      ('Chris’s iPhone', 'NoFlex for iOS', PhosphorIconsRegular.deviceMobile),
      ('iPad', 'NoFlex for iOS', PhosphorIconsRegular.deviceTablet),
      ('samsung SM-S918B', 'NoFlex for Android', PhosphorIconsFill.androidLogo),
      ('DESKTOP-4F2A', 'NoFlex for Windows', PhosphorIconsFill.windowsLogo),
      ('Chris’s MacBook Pro', 'NoFlex for macOS', PhosphorIconsFill.appleLogo),
      ('Ubuntu 24.04.1 LTS', 'NoFlex for Linux', SimpleIcons.ubuntu),
      ('NixOS 25.11', 'NoFlex for Linux', SimpleIcons.nixos),
      ('Some Distro', 'NoFlex for Linux', PhosphorIconsFill.linuxLogo),
      ('PlayStation 5', null, SimpleIcons.playstation),
      ('Xbox Series X', null, PhosphorIconsRegular.gameController),
      ('Mystery Box', null, PhosphorIconsRegular.desktop),
      (null, null, PhosphorIconsRegular.desktop),
    ]) {
      expect(deviceIconFor(device, client), icon, reason: '$device, $client');
    }
  });

  testWidgets('a picture goes up as a small square', (tester) async {
    await tester.runAsync(() async {
      expect(await _size(await pictureForUpload(await _png(3000, 1500))), (
        1024,
        1024,
      ));
      expect(await _size(await pictureForUpload(await _png(200, 400))), (
        200,
        200,
      ));
      // Left for the server, which can read more than the platform can.
      final unreadable = Uint8List.fromList(utf8.encode('an AVIF, say'));
      expect(base64Decode(await pictureForUpload(unreadable)), unreadable);
    });
  });
}
