// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';

import '../../di.dart';
import '../../theme.dart';
import '../../widgets/nf_overlays.dart';
import '../connect/connect_service.dart';
import '../player/quality.dart';
import 'cast_service.dart';

const _stats = 'stats';

/// A Cast device plays what this app asks for, so its quality is chosen here,
/// beside stream info on its screen: the web's menu on the now-playing bar.
Future<void> chooseCastSettings(RemoteDevice device) async {
  final cast = getIt<CastService>();
  final current = cast.choice.peek();
  final picked = await pickOption<String>(
    title: 'Playing on ${device.name}',
    selected: encodeChoice(current),
    options: [
      if (cast.ownReceiver)
        NfOption(
          _stats,
          cast.statsOnTv.peek()
              ? 'Hide stream info on the TV'
              : 'Show stream info on the TV',
          leading: const Icon(
            PhosphorIconsRegular.info,
            size: 20,
            color: NfColors.text2,
          ),
        ),
      NfOption(encodeChoice(QualityChoice.original), 'Original'),
      NfOption(
        encodeChoice(QualityChoice.auto),
        'Auto',
        detail: 'Adapts to the connection',
      ),
      for (final step in cast.qualities.peek())
        NfOption(
          encodeChoice(QualityChoice.pinned(step.bitrateBps)),
          step.label,
          detail: megabits(step.bitrateBps),
        ),
    ],
  );
  if (picked == null) return;
  if (picked == _stats) {
    cast.showStats(!cast.statsOnTv.peek());
  } else {
    await cast.setQuality(decodeChoice(picked));
  }
}
