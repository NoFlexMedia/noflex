// SPDX-License-Identifier: AGPL-3.0-only
//
// Sessions and devices only record free text, so this reads it for the likeliest
// match. A port of `lib/ts/device.ts`, so a change there comes here too.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:simple_icons/simple_icons.dart';

RegExp _text(String pattern) => RegExp(pattern, caseSensitive: false);

/// First match wins: a TV running Android reads as a TV, an iPad as a tablet, and a
/// browser as itself rather than as whatever it runs on. Phosphor's own logos stand in
/// where Simple Icons has none or draws too much for this size, filled to sit with its
/// solid marks. What isn't a brand stays an outline.
///
/// Without [appleMarks], Apple's devices and Safari get the plain icon for what they
/// are: App Review rejects apps that draw Apple's marks.
List<(RegExp, IconData)> _deviceIcons({required bool appleMarks}) => [
  (_text(r'\bgoogle ?tv\b'), SimpleIcons.googletv),
  (
    _text(r'\bapple ?tv\b|\btvos\b'),
    appleMarks ? PhosphorIconsFill.appleLogo : PhosphorIconsRegular.television,
  ),
  (_text(r'\bshield\b|\bnvidia\b'), SimpleIcons.nvidia),
  (
    _text(
      r'\b(tv|roku|fire ?stick|chromecast|bravia|vidaa)\b|\bwebos\b|\btizen\b',
    ),
    PhosphorIconsRegular.television,
  ),
  (_text('playstation'), SimpleIcons.playstation),
  (_text('steam ?deck'), SimpleIcons.steamdeck),
  (_text('xbox|nintendo'), PhosphorIconsRegular.gameController),
  (_text(r'\bbrave\b'), SimpleIcons.brave),
  (_text(r'\bvivaldi\b'), SimpleIcons.vivaldi),
  (_text(r'\bopera\b'), SimpleIcons.opera),
  (_text('duckduckgo'), SimpleIcons.duckduckgo),
  (_text('firefox'), SimpleIcons.firefoxbrowser),
  (
    _text('safari'),
    appleMarks ? SimpleIcons.safari : PhosphorIconsRegular.browser,
  ),
  (_text('chrome|chromium'), SimpleIcons.googlechrome),
  (_text(r'\bedge\b|samsung internet|browser'), PhosphorIconsRegular.browser),
  (_text('ipad|tablet'), PhosphorIconsRegular.deviceTablet),
  (_text(r'iphone|\bios\b|phone|mobile'), PhosphorIconsRegular.deviceMobile),
  (
    _text(r'\bmac|\bimac\b'),
    appleMarks ? PhosphorIconsFill.appleLogo : PhosphorIconsRegular.desktop,
  ),
  (_text('windows'), PhosphorIconsFill.windowsLogo),
  (_text('ubuntu'), SimpleIcons.ubuntu),
  (_text('fedora'), SimpleIcons.fedora),
  (_text('arch linux'), SimpleIcons.archlinux),
  (_text('debian'), SimpleIcons.debian),
  (_text('nixos'), SimpleIcons.nixos),
  (_text('linux mint'), SimpleIcons.linuxmint),
  (_text('pop!_os'), SimpleIcons.popos),
  (_text('manjaro'), SimpleIcons.manjaro),
  (_text('opensuse'), SimpleIcons.opensuse),
  (_text('linux'), PhosphorIconsFill.linuxLogo),
  (_text('android'), PhosphorIconsFill.androidLogo),
  (_text('laptop'), PhosphorIconsRegular.laptop),
];

final _branded = _deviceIcons(appleMarks: true);
final _unbranded = _deviceIcons(appleMarks: false);

IconData deviceIconFor(String? device, [String? client]) {
  final text = '${device ?? ''} ${client ?? ''}';
  final icons = switch (defaultTargetPlatform) {
    TargetPlatform.iOS || TargetPlatform.macOS => _unbranded,
    _ => _branded,
  };
  for (final (pattern, icon) in icons) {
    if (pattern.hasMatch(text)) return icon;
  }
  return PhosphorIconsRegular.desktop;
}
