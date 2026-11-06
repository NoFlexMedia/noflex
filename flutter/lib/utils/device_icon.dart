// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/widgets.dart';
import 'package:phosphor_icons/phosphor_icons.dart';

/// Sessions only record free text, so this reads it for the likeliest match.
/// First match wins, so a TV running Android reads as a TV and an iPad as a
/// tablet.
final _deviceIcons = <(RegExp, IconData)>[
  (
    RegExp(
      r'\b(tv|roku|fire ?stick|chromecast|shield)\b|tvos|webos|tizen',
      caseSensitive: false,
    ),
    PhosphorIconsRegular.television,
  ),
  (
    RegExp(r'playstation|xbox|nintendo|steam deck', caseSensitive: false),
    PhosphorIconsRegular.gameController,
  ),
  (
    RegExp('chrome', caseSensitive: false),
    PhosphorIconsRegular.googleChromeLogo,
  ),
  // Phosphor has no Firefox logo; its flame is the closest thing.
  (RegExp('firefox', caseSensitive: false), PhosphorIconsRegular.fire),
  (RegExp('safari', caseSensitive: false), PhosphorIconsRegular.compass),
  (
    RegExp(r'\bedge\b|browser', caseSensitive: false),
    PhosphorIconsRegular.browser,
  ),
  (
    RegExp('ipad|tablet', caseSensitive: false),
    PhosphorIconsRegular.deviceTablet,
  ),
  (
    RegExp(r'iphone|\bios\b|phone|mobile', caseSensitive: false),
    PhosphorIconsRegular.deviceMobile,
  ),
  (RegExp('android', caseSensitive: false), PhosphorIconsRegular.androidLogo),
  (
    RegExp('mac ?book|laptop', caseSensitive: false),
    PhosphorIconsRegular.laptop,
  ),
  (
    RegExp(r'\b(macos|mac)\b', caseSensitive: false),
    PhosphorIconsRegular.appleLogo,
  ),
  (RegExp('windows', caseSensitive: false), PhosphorIconsRegular.windowsLogo),
  (RegExp('linux', caseSensitive: false), PhosphorIconsRegular.linuxLogo),
];

IconData deviceIconFor(String? device, [String? client]) {
  final text = '${device ?? ''} ${client ?? ''}';
  for (final (pattern, icon) in _deviceIcons) {
    if (pattern.hasMatch(text)) return icon;
  }
  return PhosphorIconsRegular.desktop;
}
