// SPDX-License-Identifier: AGPL-3.0-only
//
// Toasts, confirmation and option sheets. Rendered through NavService, so any
// service can raise them without a BuildContext.

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../di.dart';
import '../services/nav_service.dart';
import '../services/toast_service.dart';
import '../theme.dart';
import 'nf_button.dart';

class NfToastHost extends SignalWidget {
  const NfToastHost({super.key});

  @override
  Widget build(BuildContext context) {
    final service = getIt<ToastService>();
    final toasts = service.toasts.value;
    return Positioned(
      left: 16,
      right: 16,
      bottom: 88 + MediaQuery.paddingOf(context).bottom,
      child: IgnorePointer(
        ignoring: toasts.isEmpty,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [for (final toast in toasts) _ToastCard(toast, service)],
        ),
      ),
    );
  }
}

class _ToastCard extends StatelessWidget {
  const _ToastCard(this.toast, this.service);
  final Toast toast;
  final ToastService service;

  @override
  Widget build(BuildContext context) {
    final (icon, color) = switch (toast.kind) {
      ToastKind.success => (PhosphorIconsFill.checkCircle, NfColors.success),
      ToastKind.error => (PhosphorIconsFill.warningCircle, NfColors.danger),
      ToastKind.info => (PhosphorIconsRegular.info, NfColors.text2),
    };
    return Padding(
      padding: const EdgeInsets.only(top: 8),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 480),
        child: Material(
          color: NfColors.bgRaised,
          elevation: 8,
          shadowColor: Colors.black,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(NfRadii.m),
            side: const BorderSide(color: NfColors.lineStrong),
          ),
          child: Padding(
            padding: const EdgeInsets.fromLTRB(14, 12, 10, 12),
            child: Row(
              children: [
                Icon(icon, size: 18, color: color),
                const SizedBox(width: 10),
                Expanded(child: Text(toast.message, style: nfText(14))),
                if (toast.actionLabel != null)
                  TextButton(
                    onPressed: () {
                      service.dismiss(toast.id);
                      toast.action?.call();
                    },
                    child: Text(
                      toast.actionLabel!,
                      style: nfText(13, weight: 600, color: NfColors.accent),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

Future<bool> confirmAction({
  required String title,
  required String message,
  required String confirmLabel,
  bool danger = false,
}) async {
  final result = await getIt<NavService>().openSheet<bool>(
    (context) => Padding(
      padding: const EdgeInsets.all(22),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(title, style: NfType.sectionTitle),
          const SizedBox(height: 8),
          Text(message, style: NfType.bodyMuted),
          const SizedBox(height: 20),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              NfButton(
                label: 'Cancel',
                style: NfButtonStyle.ghost,
                onPressed: () => Navigator.of(context).pop(false),
              ),
              const SizedBox(width: 10),
              NfButton(
                label: confirmLabel,
                style: danger ? NfButtonStyle.danger : NfButtonStyle.primary,
                onPressed: () => Navigator.of(context).pop(true),
              ),
            ],
          ),
        ],
      ),
    ),
  );
  return result ?? false;
}

class NfOption<T> {
  const NfOption(this.value, this.label, {this.detail, this.leading});
  final T value;
  final String label;
  final String? detail;
  final Widget? leading;
}

/// Choose one of [options] from a sheet. Completes with null when dismissed.
Future<T?> pickOption<T>({
  required String title,
  required List<NfOption<T>> options,
  T? selected,
}) => getIt<NavService>().openSheet<T>(
  (context) => Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      Padding(
        padding: const EdgeInsets.fromLTRB(22, 20, 22, 8),
        child: Text(title, style: NfType.sectionTitle),
      ),
      Flexible(
        child: ListView(
          shrinkWrap: true,
          padding: const EdgeInsets.only(bottom: 16),
          children: [
            for (final option in options)
              InkWell(
                onTap: () => Navigator.of(context).pop(option.value),
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 22,
                    vertical: 12,
                  ),
                  child: Row(
                    children: [
                      if (option.leading != null) ...[
                        option.leading!,
                        const SizedBox(width: 12),
                      ],
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(option.label, style: NfType.body),
                            if (option.detail != null)
                              Text(option.detail!, style: NfType.caption),
                          ],
                        ),
                      ),
                      if (option.value == selected)
                        const Icon(
                          PhosphorIconsBold.check,
                          size: 18,
                          color: NfColors.accent,
                        ),
                    ],
                  ),
                ),
              ),
          ],
        ),
      ),
    ],
  ),
);
