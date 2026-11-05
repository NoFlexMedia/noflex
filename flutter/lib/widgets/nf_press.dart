// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../theme.dart';

/// Whether a control is held. Feed it `InkWell.onHighlightChanged`, which a
/// scroll taking over the gesture also ends.
final class Press {
  static const sink = Duration(milliseconds: 90);

  final down = signal(false);
  final _held = Stopwatch();
  Timer? _release;

  void update(bool pressed) {
    _release?.cancel();
    if (pressed) {
      _held
        ..reset()
        ..start();
      down.value = true;
      return;
    }
    // A quick tap lifts within a frame of landing, which would show nothing.
    final left = sink - _held.elapsed;
    if (left <= Duration.zero) {
      down.value = false;
    } else {
      _release = Timer(left, () => down.value = false);
    }
  }

  void dispose() => _release?.cancel();
}

/// Sinks [child] while [press] is held, and springs it back on release.
class NfSink extends StatelessWidget {
  const NfSink({
    super.key,
    required this.press,
    required this.child,
    this.scale = 0.965,
  });

  final Press press;
  final double scale;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final still = MediaQuery.disableAnimationsOf(context);
    return SignalBuilder(
      builder: (_) {
        final down = press.down.value;
        return AnimatedScale(
          scale: down ? scale : 1,
          duration: still
              ? Duration.zero
              : down
              ? Press.sink
              : const Duration(milliseconds: 220),
          curve: down ? NfMotion.easeOut : NfMotion.spring,
          child: child,
        );
      },
    );
  }
}

/// A control that sinks as a whole. [builder] gets what its `InkWell` should
/// call for `onHighlightChanged`.
class NfPressable extends StatefulWidget {
  const NfPressable({super.key, required this.builder, this.scale = 0.965});

  final double scale;
  final Widget Function(BuildContext context, ValueChanged<bool> onPress)
  builder;

  @override
  State<NfPressable> createState() => _NfPressableState();
}

class _NfPressableState extends State<NfPressable> {
  final _press = Press();

  @override
  void dispose() {
    _press.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => NfSink(
    press: _press,
    scale: widget.scale,
    child: widget.builder(context, _press.update),
  );
}
