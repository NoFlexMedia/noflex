// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';

import '../theme.dart';
import 'nf_button.dart';

/// An empty or failed page: icon, title, message, and what to do about it.
class NfState extends StatelessWidget {
  const NfState({
    super.key,
    required this.title,
    this.message,
    this.icon,
    this.actionLabel,
    this.onAction,
    this.isError = false,
    this.child,
  });

  const NfState.error({
    super.key,
    required this.title,
    this.message,
    required VoidCallback retry,
  }) : icon = PhosphorIconsRegular.warningCircle,
       actionLabel = 'Try again',
       onAction = retry,
       isError = true,
       child = null;

  final String title;
  final String? message;
  final IconData? icon;
  final String? actionLabel;
  final VoidCallback? onAction;
  final bool isError;
  final Widget? child;

  @override
  Widget build(BuildContext context) => Center(
    child: Padding(
      padding: const EdgeInsets.all(28),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 420),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (icon != null) ...[
              Container(
                width: 56,
                height: 56,
                decoration: BoxDecoration(
                  color: isError ? NfColors.dangerSoft : NfColors.surface2,
                  borderRadius: BorderRadius.circular(16),
                ),
                child: Icon(
                  icon,
                  size: 26,
                  color: isError ? NfColors.danger : NfColors.text2,
                ),
              ),
              const SizedBox(height: 16),
            ],
            Text(
              title,
              textAlign: TextAlign.center,
              style: NfType.sectionTitle,
            ),
            if (message != null) ...[
              const SizedBox(height: 8),
              Text(
                message!,
                textAlign: TextAlign.center,
                style: NfType.bodyMuted,
              ),
            ],
            if (actionLabel != null) ...[
              const SizedBox(height: 18),
              NfButton(label: actionLabel!, onPressed: onAction),
            ],
            ?child,
          ],
        ),
      ),
    ),
  );
}

/// A placeholder block while content loads, in the shape of what's coming.
class NfSkeleton extends StatefulWidget {
  const NfSkeleton({
    super.key,
    this.width,
    this.height,
    this.radius = NfRadii.m,
  });

  final double? width;
  final double? height;
  final double radius;

  @override
  State<NfSkeleton> createState() => _NfSkeletonState();
}

class _NfSkeletonState extends State<NfSkeleton>
    with SingleTickerProviderStateMixin {
  late final _sweep = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1600),
  );

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _sweep.stop();
    } else if (!_sweep.isAnimating) {
      _sweep.repeat();
    }
  }

  @override
  void dispose() {
    _sweep.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => RepaintBoundary(
    child: ClipRRect(
      borderRadius: BorderRadius.circular(widget.radius),
      child: CustomPaint(
        foregroundPainter: _SweepPainter(_sweep),
        child: Container(
          width: widget.width,
          height: widget.height,
          color: NfColors.surface2,
        ),
      ),
    ),
  );
}

class _SweepPainter extends CustomPainter {
  _SweepPainter(this.sweep) : super(repaint: sweep);

  final Animation<double> sweep;

  @override
  void paint(Canvas canvas, Size size) {
    if (!sweep.isAnimating) return;
    final band = Offset(size.width * (2 * sweep.value - 1), 0) & size;
    canvas.drawRect(
      band,
      Paint()
        ..shader = const LinearGradient(
          colors: [Colors.transparent, NfColors.surface2, Colors.transparent],
        ).createShader(band),
    );
  }

  @override
  bool shouldRepaint(_SweepPainter oldDelegate) => false;
}

/// Rows of placeholder, for a list or section still loading.
class NfSkeletonRows extends StatelessWidget {
  const NfSkeletonRows({super.key, this.count = 4, this.height = 56});

  final int count;
  final double height;

  @override
  Widget build(BuildContext context) => Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      for (var i = 0; i < count; i++)
        Padding(
          padding: EdgeInsets.only(top: i == 0 ? 0 : 10),
          child: NfSkeleton(height: height),
        ),
    ],
  );
}

class NfProgressBar extends StatelessWidget {
  const NfProgressBar({
    super.key,
    required this.value,
    this.height = 4,
    this.color = NfColors.accent,
    this.track = NfColors.surface3,
  });

  /// 0 to 1; null for indeterminate.
  final double? value;
  final double height;
  final Color color;
  final Color track;

  @override
  Widget build(BuildContext context) => ClipRRect(
    borderRadius: BorderRadius.circular(height),
    child: LinearProgressIndicator(
      value: value,
      minHeight: height,
      color: color,
      backgroundColor: track,
    ),
  );
}

/// A section title with an optional trailing action.
class NfSectionHead extends StatelessWidget {
  const NfSectionHead(
    this.title, {
    super.key,
    this.eyebrow,
    this.trailing,
    this.onTap,
  });

  final String title;
  final String? eyebrow;
  final Widget? trailing;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final heading = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (eyebrow != null) ...[
          Text(eyebrow!.toUpperCase(), style: NfType.eyebrow),
          const SizedBox(height: 3),
        ],
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Flexible(
              child: Text(
                title,
                style: NfType.sectionTitle,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            if (onTap != null)
              const Padding(
                padding: EdgeInsets.only(left: 4),
                child: Icon(
                  PhosphorIconsRegular.caretRight,
                  size: 18,
                  color: NfColors.text2,
                ),
              ),
          ],
        ),
      ],
    );
    return Row(
      crossAxisAlignment: CrossAxisAlignment.end,
      children: [
        Expanded(
          child: onTap == null
              ? heading
              : InkWell(
                  onTap: onTap,
                  borderRadius: BorderRadius.circular(8),
                  child: heading,
                ),
        ),
        ?trailing,
      ],
    );
  }
}

/// A rounded panel, the web's `.panel`.
class NfPanel extends StatelessWidget {
  const NfPanel({
    super.key,
    required this.child,
    this.padding = const EdgeInsets.all(16),
    this.onTap,
  });

  final Widget child;
  final EdgeInsets padding;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) => Material(
    color: NfColors.surface,
    shape: RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(NfRadii.l),
      side: const BorderSide(color: NfColors.line),
    ),
    clipBehavior: Clip.antiAlias,
    child: InkWell(
      onTap: onTap,
      child: Padding(padding: padding, child: child),
    ),
  );
}
