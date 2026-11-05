// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';

import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_state.dart';

/// A titled block of settings, the web's `.panel` with its `.panel-head`.
class AccountPanel extends StatelessWidget {
  const AccountPanel({
    super.key,
    required this.icon,
    required this.title,
    required this.child,
    this.detail,
    this.trailing,
  });

  final IconData icon;
  final String title;
  final String? detail;
  final Widget? trailing;
  final Widget child;

  @override
  Widget build(BuildContext context) => NfPanel(
    padding: const EdgeInsets.all(18),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            AccountGlyph(icon: icon),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(title, style: nfText(15.5, weight: 650)),
                  if (detail != null) ...[
                    const SizedBox(height: 2),
                    Text(detail!, style: NfType.caption),
                  ],
                ],
              ),
            ),
            ?trailing,
          ],
        ),
        const SizedBox(height: 16),
        child,
      ],
    ),
  );
}

class AccountGlyph extends StatelessWidget {
  const AccountGlyph({
    super.key,
    required this.icon,
    this.highlighted = false,
    this.color = NfColors.text2,
    this.background = NfColors.surface2,
  });

  final IconData icon;
  final bool highlighted;
  final Color color;
  final Color background;

  @override
  Widget build(BuildContext context) => Container(
    width: 38,
    height: 38,
    decoration: BoxDecoration(
      color: highlighted ? NfColors.accentSoft : background,
      borderRadius: BorderRadius.circular(NfRadii.m),
    ),
    child: Icon(icon, size: 18, color: highlighted ? NfColors.accent : color),
  );
}

/// One thing in a panel's list: a device, a session, a server.
class AccountRow extends StatelessWidget {
  const AccountRow({
    super.key,
    required this.title,
    this.icon,
    this.isCurrent = false,
    this.currentLabel,
    this.detail,
    this.trailing,
  });

  final String title;
  final IconData? icon;
  final bool isCurrent;
  final String? currentLabel;
  final String? detail;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 8),
    child: Row(
      children: [
        if (icon != null) ...[
          AccountGlyph(icon: icon!, highlighted: isCurrent),
          const SizedBox(width: 12),
        ],
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Wrap(
                spacing: 8,
                runSpacing: 4,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  Text(title, style: nfText(14.5, weight: 600)),
                  if (isCurrent && currentLabel != null)
                    NfBadge(
                      currentLabel!,
                      color: NfColors.accent,
                      background: NfColors.accentSoft,
                    ),
                ],
              ),
              if (detail != null) ...[
                const SizedBox(height: 2),
                Text(detail!, style: NfType.caption),
              ],
            ],
          ),
        ),
        if (trailing != null) ...[const SizedBox(width: 8), trailing!],
      ],
    ),
  );
}

/// A setting that's on or off, with its explanation.
class AccountToggle extends StatelessWidget {
  const AccountToggle({
    super.key,
    required this.title,
    required this.value,
    required this.onChanged,
    this.detail,
  });

  final String title;
  final String? detail;
  final bool value;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 4),
    child: Row(
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: nfText(14.5, weight: 550)),
              if (detail != null) ...[
                const SizedBox(height: 2),
                Text(detail!, style: NfType.caption),
              ],
            ],
          ),
        ),
        const SizedBox(width: 12),
        Switch(value: value, onChanged: onChanged),
      ],
    ),
  );
}

/// Panels stacked to a readable width, however wide the window is.
class AccountColumn extends StatelessWidget {
  const AccountColumn({super.key, required this.children});
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Align(
    alignment: Alignment.topLeft,
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: accountMaxWidth),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (index, child) in children.indexed) ...[
            if (index > 0) const SizedBox(height: 14),
            child,
          ],
        ],
      ),
    ),
  );
}

const accountMaxWidth = 680.0;
