// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';

import '../di.dart';
import '../services/indexing_service.dart';
import '../theme.dart';

/// Over a title the server is still working on. In place of art it doesn't have
/// yet it's a tag with a slow sheen, and over art it does have, a small pill. The
/// web's `Artwork` is the reference.
class IndexingOverlay extends StatefulWidget {
  const IndexingOverlay({
    super.key,
    required this.workId,
    required this.stage,
    required this.hasArt,
    required this.wide,
  });

  final String workId;
  final String stage;
  final bool hasArt;

  /// A wide card's title sits along its bottom, so this keeps to the top corner.
  final bool wide;

  @override
  State<IndexingOverlay> createState() => _IndexingOverlayState();
}

class _IndexingOverlayState extends State<IndexingOverlay>
    with SingleTickerProviderStateMixin {
  late final _motion = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 2400),
  );
  late void Function() _stopWatching;

  @override
  void initState() {
    super.initState();
    _stopWatching = getIt<IndexingService>().watch(widget.workId);
  }

  @override
  void didUpdateWidget(IndexingOverlay old) {
    super.didUpdateWidget(old);
    if (old.workId == widget.workId) return;
    _stopWatching();
    _stopWatching = getIt<IndexingService>().watch(widget.workId);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (MediaQuery.disableAnimationsOf(context)) {
      _motion.stop();
    } else if (!_motion.isAnimating) {
      _motion.repeat();
    }
  }

  @override
  void dispose() {
    _stopWatching();
    _motion.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final label = indexingLabels[widget.stage] ?? 'Indexing';
    return Semantics(
      label: label,
      liveRegion: true,
      child: ExcludeSemantics(
        child: widget.hasArt ? _pill(label) : _placeholder(label),
      ),
    );
  }

  Widget _pill(String label) => Align(
    alignment: Alignment.topLeft,
    child: Padding(
      padding: const EdgeInsets.all(8),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: const Color(0xB3000000),
          borderRadius: BorderRadius.circular(NfRadii.pill),
        ),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(8, 4, 10, 4),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              FadeTransition(
                opacity: _motion.drive(
                  TweenSequence([
                    TweenSequenceItem(
                      tween: Tween(begin: 1, end: 0.4),
                      weight: 1,
                    ),
                    TweenSequenceItem(
                      tween: Tween(begin: 0.4, end: 1),
                      weight: 1,
                    ),
                  ]),
                ),
                child: Icon(
                  PhosphorIconsRegular.tag,
                  size: 14,
                  color: NfColors.text,
                ),
              ),
              const SizedBox(width: 6),
              Flexible(child: _Label(label, color: NfColors.text)),
            ],
          ),
        ),
      ),
    ),
  );

  Widget _placeholder(String label) => LayoutBuilder(
    builder: (context, constraints) {
      final width = constraints.maxWidth;
      final tag = _PulsingTag(motion: _motion);
      final Widget content;
      if (widget.wide) {
        content = Align(
          alignment: Alignment.topLeft,
          child: Padding(
            padding: const EdgeInsets.all(14),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                tag,
                const SizedBox(width: 10),
                Flexible(child: _Label(label, color: NfColors.text2)),
              ],
            ),
          ),
        );
      } else {
        content = Center(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              tag,
              // A tiny poster, as in a list row, has room for the tag alone.
              if (width >= 110) ...[
                const SizedBox(height: 10),
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  child: _Label(label, color: NfColors.text2),
                ),
              ],
            ],
          ),
        );
      }
      return Stack(
        fit: StackFit.expand,
        children: [
          AnimatedBuilder(
            animation: _motion,
            builder: (_, _) => FractionalTranslation(
              translation: Offset(_motion.value * 2 - 1, 0),
              child: const DecoratedBox(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    stops: [0.3, 0.5, 0.7],
                    colors: [
                      Color(0x00FFFFFF),
                      Color(0x17FFFFFF),
                      Color(0x00FFFFFF),
                    ],
                  ),
                ),
              ),
            ),
          ),
          content,
        ],
      );
    },
  );
}

class _PulsingTag extends StatelessWidget {
  const _PulsingTag({required this.motion});

  final Animation<double> motion;

  @override
  Widget build(BuildContext context) {
    const size = 48.0;
    return SizedBox.square(
      dimension: size,
      child: Stack(
        fit: StackFit.expand,
        clipBehavior: Clip.none,
        children: [
          AnimatedBuilder(
            animation: motion,
            builder: (_, child) {
              final t = NfMotion.easeOut.transform(motion.value);
              return Opacity(
                opacity: (0.55 * (1 - t)).clamp(0, 1),
                child: Transform.scale(scale: 1 + 0.55 * t, child: child),
              );
            },
            child: DecoratedBox(
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                border: Border.all(color: NfColors.text2, width: 1.5),
              ),
            ),
          ),
          DecoratedBox(
            decoration: const BoxDecoration(
              shape: BoxShape.circle,
              color: NfColors.surface3,
            ),
            child: Icon(
              PhosphorIconsRegular.tag,
              size: 22,
              color: NfColors.text,
            ),
          ),
        ],
      ),
    );
  }
}

class _Label extends StatelessWidget {
  const _Label(this.text, {required this.color});

  final String text;
  final Color color;

  @override
  Widget build(BuildContext context) => Text(
    text,
    maxLines: 1,
    overflow: TextOverflow.ellipsis,
    textAlign: TextAlign.center,
    style: NfType.caption.copyWith(fontWeight: FontWeight.w600, color: color),
  );
}
