// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../services/device_identity.dart';
import '../../theme.dart';
import '../../utils/device_icon.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import 'connect_service.dart';

// The sheet's value for this device, as an option needs one.
const _here = '';

/// Choose where Play goes: this device, or one of the viewer's others that is
/// listening.
Future<void> choosePlayTarget() async {
  final connect = getIt<ConnectService>();
  final identity = getIt<DeviceIdentity>();
  Widget icon(String? name, String? client) =>
      Icon(deviceIconFor(name, client), size: 20, color: NfColors.text2);
  final chosen = await pickOption<String>(
    title: 'Play on',
    selected: connect.targetId.peek() ?? _here,
    options: [
      NfOption(
        _here,
        'This device',
        leading: icon(identity.name, identity.clientName),
      ),
      for (final device in connect.available.peek())
        NfOption(
          device.id,
          device.name,
          leading: icon(device.name, device.client),
        ),
    ],
  );
  if (chosen != null) connect.setTarget(chosen == _here ? null : chosen);
}

/// Offered only while there is somewhere to play, as a cast button is.
class ConnectButton extends SignalWidget {
  const ConnectButton({super.key});

  @override
  Widget build(BuildContext context) {
    final connect = getIt<ConnectService>();
    final target = connect.target.value;
    if (target == null && connect.available.value.isEmpty) {
      return const SizedBox.shrink();
    }
    return NfIconButton.glass(
      icon: target == null
          ? PhosphorIconsRegular.monitorPlay
          : PhosphorIconsFill.monitorPlay,
      label: target == null ? 'Play on a device' : 'Playing on ${target.name}',
      color: target == null ? NfColors.text : NfColors.accent,
      onPressed: choosePlayTarget,
    );
  }
}
