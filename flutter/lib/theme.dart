// SPDX-License-Identifier: AGPL-3.0-only
//
// Design tokens, ported from web/src/app.css. NoFlex is dark only, as on the
// web, so these are plain constants rather than a ThemeExtension.

import 'package:flutter/cupertino.dart' show CupertinoPageTransitionsBuilder;
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';

abstract final class NfColors {
  static const bg = Color(0xFF07080B);
  static const bgRaised = Color(0xFF0C0E12);
  static const bgOverlay = Color(0xC7090A0E);
  static const sidebar = Color(0xFF08090C);
  static const surface = Color(0x09FFFFFF);
  static const surface2 = Color(0x0FFFFFFF);
  static const surface3 = Color(0x1AFFFFFF);
  static const line = Color(0x13FFFFFF);
  static const lineStrong = Color(0x24FFFFFF);
  static const text = Color(0xFFF4F5F1);
  static const text2 = Color(0xB3F4F5F1);
  static const text3 = Color(0x75F4F5F1);
  static const accent = Color(0xFFD4FF5A);
  static const accentSoft = Color(0x24D4FF5A);
  static const accentInk = Color(0xFF10150A);
  static const danger = Color(0xFFFF6B6B);
  static const dangerSoft = Color(0x1FFF6B6B);
  static const warning = Color(0xFFFFC164);
  static const success = Color(0xFF6EE7A8);
  static const successSoft = Color(0x1F6EE7A8);
  static const focus = Color(0xFFA5DCFF);
  static const heart = Color(0xFFFF6B8B);

  /// A page's tint until its artwork gives it one.
  static const ambient = Color(0xFF5A688C);
}

abstract final class NfRadii {
  static const xs = 6.0;
  static const s = 9.0;
  static const m = 13.0;
  static const l = 18.0;
  static const xl = 26.0;
  static const pill = 999.0;
}

abstract final class NfMotion {
  static const easeOut = Cubic(0.16, 1, 0.3, 1);
  static const easeInOut = Cubic(0.65, 0, 0.35, 1);
  static const spring = _Spring();
  static const fast = Duration(milliseconds: 140);
  static const base = Duration(milliseconds: 240);
  static const slow = Duration(milliseconds: 520);
}

/// `--ease-spring`, stop for stop.
final class _Spring extends Curve {
  const _Spring();

  static const _stops = [
    (0.0, 0.0),
    (0.024, 0.06),
    (0.052, 0.24),
    (0.13, 0.86),
    (0.176, 1.04),
    (0.21, 1.09),
    (0.254, 1.07),
    (0.33, 1.01),
    (0.41, 0.99),
    (1.0, 1.0),
  ];

  @override
  double transformInternal(double t) {
    for (var i = 1; i < _stops.length; i++) {
      final (x, y) = _stops[i];
      if (t > x) continue;
      final (fromX, fromY) = _stops[i - 1];
      return fromY + (y - fromY) * (t - fromX) / (x - fromX);
    }
    return 1;
  }
}

abstract final class NfSpacing {
  static const railWidth = 76.0;
  static const sidebarWidth = 236.0;
  static const bottomNavHeight = 64.0;
}

/// Inter is a variable font, so weight has to travel as a font variation:
/// `fontWeight` alone would leave the glyphs at the file's default.
TextStyle nfText(
  double size, {
  int weight = 400,
  Color color = NfColors.text,
  double? height,
  double letterSpacing = -0.09,
  FontStyle? fontStyle,
  List<FontFeature>? features,
}) => TextStyle(
  fontFamily: 'Inter',
  fontSize: size,
  fontWeight: FontWeight.values[((weight / 100).round() - 1).clamp(0, 8)],
  fontVariations: [FontVariation('wght', weight.toDouble())],
  color: color,
  height: height,
  letterSpacing: letterSpacing,
  fontStyle: fontStyle,
  fontFeatures: features,
);

abstract final class NfType {
  static final eyebrow = nfText(
    11,
    weight: 600,
    color: NfColors.text3,
    letterSpacing: 1.54,
  );
  static final display = nfText(
    34,
    weight: 750,
    letterSpacing: -1.5,
    height: 1.05,
  );
  static final pageTitle = nfText(
    28,
    weight: 650,
    letterSpacing: -0.62,
    height: 1.15,
  );
  static final sectionTitle = nfText(
    18,
    weight: 650,
    letterSpacing: -0.4,
    height: 1.15,
  );
  static final body = nfText(15, height: 1.5);
  static final bodyMuted = nfText(15, color: NfColors.text2, height: 1.5);
  static final small = nfText(13, color: NfColors.text2, height: 1.4);
  static final caption = nfText(12, color: NfColors.text3, height: 1.35);
  static final label = nfText(14, weight: 600);
  static final mono = const TextStyle(
    fontFamily: 'monospace',
    fontSize: 12.5,
    color: NfColors.text2,
  );
  static const tabular = [FontFeature.tabularFigures()];
}

ThemeData buildNoFlexTheme() {
  const scheme = ColorScheme.dark(
    surface: NfColors.bg,
    primary: NfColors.accent,
    onPrimary: NfColors.accentInk,
    secondary: NfColors.accent,
    onSecondary: NfColors.accentInk,
    error: NfColors.danger,
    onSurface: NfColors.text,
    surfaceContainerHighest: NfColors.bgRaised,
    outline: NfColors.lineStrong,
  );
  return ThemeData(
    brightness: Brightness.dark,
    colorScheme: scheme,
    scaffoldBackgroundColor: NfColors.bg,
    canvasColor: NfColors.bg,
    fontFamily: 'Inter',
    splashFactory: InkSparkle.splashFactory,
    // Material draws these from its own font unless told otherwise.
    actionIconTheme: ActionIconThemeData(
      backButtonIconBuilder: (_) => const Icon(PhosphorIconsRegular.arrowLeft),
      closeButtonIconBuilder: (_) => const Icon(PhosphorIconsRegular.x),
      drawerButtonIconBuilder: (_) => const Icon(PhosphorIconsRegular.list),
      endDrawerButtonIconBuilder: (_) => const Icon(PhosphorIconsRegular.list),
    ),
    highlightColor: Colors.transparent,
    hoverColor: NfColors.surface2,
    focusColor: NfColors.focus.withValues(alpha: 0.3),
    dividerColor: NfColors.line,
    dividerTheme: const DividerThemeData(color: NfColors.line, thickness: 1),
    textTheme: TextTheme(
      bodyMedium: NfType.body,
      bodySmall: NfType.small,
      titleMedium: NfType.label,
    ),
    textSelectionTheme: const TextSelectionThemeData(
      cursorColor: NfColors.accent,
      selectionColor: Color(0x55D4FF5A),
      selectionHandleColor: NfColors.accent,
    ),
    scrollbarTheme: ScrollbarThemeData(
      thumbColor: WidgetStateProperty.all(const Color(0x29FFFFFF)),
      thickness: WidgetStateProperty.all(6),
      radius: const Radius.circular(3),
    ),
    tooltipTheme: TooltipThemeData(
      decoration: BoxDecoration(
        color: NfColors.bgRaised,
        borderRadius: BorderRadius.circular(NfRadii.s),
        border: Border.all(color: NfColors.lineStrong),
      ),
      textStyle: NfType.small,
      waitDuration: const Duration(milliseconds: 350),
    ),
    pageTransitionsTheme: const PageTransitionsTheme(
      builders: {
        TargetPlatform.android: PredictiveBackPageTransitionsBuilder(),
        TargetPlatform.iOS: CupertinoPageTransitionsBuilder(),
        TargetPlatform.macOS: FadeForwardsPageTransitionsBuilder(),
        TargetPlatform.windows: FadeForwardsPageTransitionsBuilder(),
        TargetPlatform.linux: FadeForwardsPageTransitionsBuilder(),
      },
    ),
  );
}
