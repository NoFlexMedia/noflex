// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';

import '../theme.dart';

enum NfButtonStyle { primary, light, glass, ghost, danger, surface }

enum NfButtonSize { small, medium, large }

class NfButton extends StatelessWidget {
  const NfButton({
    super.key,
    required this.label,
    this.onPressed,
    this.icon,
    this.style = NfButtonStyle.surface,
    this.size = NfButtonSize.medium,
    this.busy = false,
    this.expand = false,
    this.semanticLabel,
  });

  final String label;
  final VoidCallback? onPressed;
  final Widget? icon;
  final NfButtonStyle style;
  final NfButtonSize size;
  final bool busy;
  final bool expand;
  final String? semanticLabel;

  @override
  Widget build(BuildContext context) {
    final (background, foreground, border) = switch (style) {
      NfButtonStyle.primary => (NfColors.accent, NfColors.accentInk, null),
      NfButtonStyle.light => (NfColors.text, const Color(0xFF0B0C0F), null),
      NfButtonStyle.glass => (
        const Color(0x24FFFFFF),
        NfColors.text,
        NfColors.lineStrong,
      ),
      NfButtonStyle.ghost => (
        Colors.transparent,
        NfColors.text2,
        NfColors.lineStrong,
      ),
      NfButtonStyle.danger => (NfColors.dangerSoft, NfColors.danger, null),
      NfButtonStyle.surface => (NfColors.surface2, NfColors.text, null),
    };
    final (height, padding, fontSize) = switch (size) {
      NfButtonSize.small => (34.0, 12.0, 13.0),
      NfButtonSize.medium => (42.0, 16.0, 14.0),
      NfButtonSize.large => (50.0, 22.0, 15.0),
    };
    final enabled = onPressed != null && !busy;
    final content = Row(
      mainAxisSize: expand ? MainAxisSize.max : MainAxisSize.min,
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        if (busy)
          SizedBox.square(
            dimension: fontSize + 2,
            child: CircularProgressIndicator(strokeWidth: 2, color: foreground),
          )
        else if (icon != null)
          IconTheme.merge(
            data: IconThemeData(color: foreground, size: fontSize + 4),
            child: icon!,
          ),
        if (busy || icon != null) const SizedBox(width: 8),
        Flexible(
          child: Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: nfText(
              fontSize,
              weight: 600,
              color: foreground,
              letterSpacing: -0.1,
            ),
          ),
        ),
      ],
    );
    return Semantics(
      button: true,
      enabled: enabled,
      label: semanticLabel ?? label,
      excludeSemantics: true,
      child: Opacity(
        opacity: enabled || busy ? 1 : 0.45,
        child: Material(
          color: background,
          shape: StadiumBorder(
            side: border == null ? BorderSide.none : BorderSide(color: border),
          ),
          clipBehavior: Clip.antiAlias,
          child: InkWell(
            onTap: enabled ? onPressed : null,
            child: Container(
              height: height,
              padding: EdgeInsets.symmetric(horizontal: padding),
              child: content,
            ),
          ),
        ),
      ),
    );
  }
}

class NfIconButton extends StatelessWidget {
  const NfIconButton({
    super.key,
    required this.icon,
    required this.label,
    this.onPressed,
    this.size = 44,
    this.iconSize = 20,
    this.color = NfColors.text,
    this.background = Colors.transparent,
    this.border,
    this.showTooltip = true,
  });

  /// A translucent button over artwork.
  const NfIconButton.glass({
    super.key,
    required this.icon,
    required this.label,
    this.onPressed,
    this.size = 50,
    this.iconSize = 20,
    this.color = NfColors.text,
    this.showTooltip = true,
  }) : background = const Color(0x24FFFFFF),
       border = NfColors.lineStrong;

  final IconData icon;
  final String label;
  final VoidCallback? onPressed;
  final double size;
  final double iconSize;
  final Color color;
  final Color background;
  final Color? border;
  final bool showTooltip;

  @override
  Widget build(BuildContext context) {
    final button = Semantics(
      button: true,
      label: label,
      enabled: onPressed != null,
      excludeSemantics: true,
      child: Opacity(
        opacity: onPressed == null ? 0.4 : 1,
        child: Material(
          color: background,
          shape: CircleBorder(
            side: border == null ? BorderSide.none : BorderSide(color: border!),
          ),
          clipBehavior: Clip.antiAlias,
          child: InkWell(
            onTap: onPressed,
            child: SizedBox.square(
              dimension: size,
              child: Icon(icon, size: iconSize, color: color),
            ),
          ),
        ),
      ),
    );
    return showTooltip ? Tooltip(message: label, child: button) : button;
  }
}

/// A selectable pill, for editions, versions and filters.
class NfChoiceChip extends StatelessWidget {
  const NfChoiceChip({
    super.key,
    required this.label,
    required this.selected,
    required this.onTap,
    this.detail,
    this.leading,
  });

  final String label;
  final String? detail;
  final bool selected;
  final VoidCallback onTap;
  final Widget? leading;

  @override
  Widget build(BuildContext context) {
    final foreground = selected ? const Color(0xFF0B0C0F) : NfColors.text2;
    return Semantics(
      selected: selected,
      button: true,
      child: Material(
        color: selected ? NfColors.text : Colors.transparent,
        shape: StadiumBorder(
          side: BorderSide(
            color: selected ? NfColors.text : NfColors.lineStrong,
          ),
        ),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 9),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (leading != null) ...[
                  IconTheme.merge(
                    data: IconThemeData(size: 14, color: foreground),
                    child: leading!,
                  ),
                  const SizedBox(width: 6),
                ],
                Flexible(
                  child: Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: nfText(13, weight: 600, color: foreground),
                  ),
                ),
                if (detail != null) ...[
                  const SizedBox(width: 8),
                  Text(
                    detail!,
                    style: nfText(
                      12,
                      weight: 500,
                      color: selected
                          ? const Color(0x8C000000)
                          : NfColors.text3,
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class NfBadge extends StatelessWidget {
  const NfBadge(
    this.label, {
    super.key,
    this.color = NfColors.text2,
    this.background = NfColors.surface2,
    this.icon,
  });

  final String label;
  final Color color;
  final Color background;
  final IconData? icon;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
    decoration: BoxDecoration(
      color: background,
      borderRadius: BorderRadius.circular(NfRadii.xs),
      border: Border.all(color: NfColors.line),
    ),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (icon != null) ...[
          Icon(icon, size: 12, color: color),
          const SizedBox(width: 4),
        ],
        Text(
          label,
          style: nfText(11.5, weight: 600, color: color, letterSpacing: 0),
        ),
      ],
    ),
  );
}
