// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../downloads/leave_account.dart';
import 'account_controller.dart';
import 'account_widgets.dart';

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

IconData _iconFor(String? device, String? client) {
  final text = '${device ?? ''} ${client ?? ''}';
  for (final (pattern, icon) in _deviceIcons) {
    if (pattern.hasMatch(text)) return icon;
  }
  return PhosphorIconsRegular.desktop;
}

/// Other clients may send a blank device name rather than none.
String? _deviceName(AccountSession session) =>
    (session.device?.isEmpty ?? true) ? null : session.device;

class DevicesSection extends StatelessWidget {
  const DevicesSection({super.key, required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) => AccountColumn(
    children: [
      _LinkDevicePanel(controller: controller),
      _SessionsPanel(controller: controller),
      _RegisteredPanel(controller: controller),
    ],
  );
}

class _LinkDevicePanel extends StatefulWidget {
  const _LinkDevicePanel({required this.controller});
  final AccountController controller;

  @override
  State<_LinkDevicePanel> createState() => _LinkDevicePanelState();
}

class _LinkDevicePanelState extends State<_LinkDevicePanel> {
  final _pin = TextEditingController();
  final _hasPin = signal(false);
  final _busy = signal(false);

  @override
  void dispose() {
    _pin.dispose();
    super.dispose();
  }

  Future<void> _approve() async {
    final pin = _pin.text.trim();
    if (pin.isEmpty || _busy.value) return;
    _busy.value = true;
    final approved = await widget.controller.approvePin(pin);
    if (!mounted) return;
    _busy.value = false;
    if (approved) {
      _pin.clear();
      _hasPin.value = false;
    }
  }

  @override
  Widget build(BuildContext context) => AccountPanel(
    icon: PhosphorIconsRegular.television,
    title: 'Link a device',
    detail: 'A new TV or phone signs in by showing a code. Approve it here.',
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.end,
      children: [
        Expanded(
          child: NfField(
            controller: _pin,
            label: 'Device PIN',
            hint: 'K7M2QX',
            textCapitalization: TextCapitalization.characters,
            textInputAction: TextInputAction.done,
            autocorrect: false,
            onChanged: (value) => _hasPin.value = value.trim().isNotEmpty,
            onSubmitted: (_) => unawaited(_approve()),
          ),
        ),
        const SizedBox(width: 10),
        SignalBuilder(
          builder: (context) => NfButton(
            label: 'Approve',
            style: NfButtonStyle.primary,
            busy: _busy.value,
            onPressed: _hasPin.value ? () => unawaited(_approve()) : null,
          ),
        ),
      ],
    ),
  );
}

class _SessionsPanel extends SignalWidget {
  const _SessionsPanel({required this.controller});
  final AccountController controller;

  Future<void> _revoke(AccountSession session) async {
    final confirmed = await confirmAction(
      title: 'Sign out “${_deviceName(session) ?? 'this device'}”?',
      message: 'It will need to sign in again to keep using NoFlex.',
      confirmLabel: 'Sign out device',
      danger: true,
    );
    if (confirmed) await controller.revokeSession(session.id);
  }

  Future<void> _revokeOthers(int count) async {
    final confirmed = await confirmAction(
      title: 'Sign out $count other ${count == 1 ? 'device' : 'devices'}?',
      message: 'Only this device stays signed in.',
      confirmLabel: 'Sign out others',
      danger: true,
    );
    if (confirmed) await controller.revokeOtherSessions();
  }

  @override
  Widget build(BuildContext context) {
    final sessions = controller.sessions.value;
    final current = controller.currentSessionId.value;
    final others = sessions.where((s) => s.id != current).length;
    return AccountPanel(
      icon: PhosphorIconsRegular.devices,
      title: 'Signed-in devices',
      detail:
          '${sessions.length} active ${sessions.length == 1 ? 'session' : 'sessions'}',
      trailing: others > 0
          ? NfButton(
              label: 'Sign out others',
              style: NfButtonStyle.ghost,
              size: NfButtonSize.small,
              onPressed: () => unawaited(_revokeOthers(others)),
            )
          : null,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (sessions.isEmpty)
            Text('No active sessions.', style: NfType.small)
          else
            for (final session in sessions)
              _SessionRow(
                session: session,
                isCurrent: session.id == current,
                onRevoke: () => unawaited(_revoke(session)),
              ),
        ],
      ),
    );
  }
}

class _SessionRow extends StatelessWidget {
  const _SessionRow({
    required this.session,
    required this.isCurrent,
    required this.onRevoke,
  });

  final AccountSession session;
  final bool isCurrent;
  final VoidCallback onRevoke;

  @override
  Widget build(BuildContext context) {
    final client = [
      ?session.client,
      if (session.appVersion != null) 'v${session.appVersion}',
    ].join(' ');
    return AccountRow(
      icon: _iconFor(session.device, session.client),
      title: _deviceName(session) ?? 'Unknown device',
      isCurrent: isCurrent,
      currentLabel: 'This device',
      detail: [
        if (client.isEmpty) 'NoFlex client' else client,
        if (isCurrent) 'active now' else 'seen ${ago(session.lastSeenAt)}',
        session.remoteAddr ?? 'local',
      ].join(' · '),
      trailing: isCurrent
          ? NfButton(
              label: 'Sign out',
              style: NfButtonStyle.ghost,
              size: NfButtonSize.small,
              onPressed: () => unawaited(signOut()),
            )
          : NfButton(
              label: 'Revoke',
              semanticLabel:
                  'Sign out ${_deviceName(session) ?? 'unknown device'}',
              style: NfButtonStyle.ghost,
              size: NfButtonSize.small,
              onPressed: onRevoke,
            ),
    );
  }
}

class _RegisteredPanel extends SignalWidget {
  const _RegisteredPanel({required this.controller});
  final AccountController controller;

  Future<void> _forget(AccountDevice device) async {
    final confirmed = await confirmAction(
      title: 'Forget “${device.name}”?',
      message: 'The server stops preparing titles for it. It registers again the next time it signs in.',
      confirmLabel: 'Forget device',
      danger: true,
    );
    if (confirmed) await controller.forgetDevice(device.id);
  }

  @override
  Widget build(BuildContext context) {
    final devices = controller.devices.value;
    return AccountPanel(
      icon: PhosphorIconsRegular.deviceMobile,
      title: 'Registered devices',
      detail: 'What the server prepares and keeps titles for',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (devices.isEmpty)
            Text(
              'This device registers itself once it has measured what it can play.',
              style: NfType.small,
            )
          else
            for (final device in devices)
              AccountRow(
                icon: _iconFor(device.name, null),
                title: device.name,
                isCurrent: controller.isThisDevice(device.id),
                currentLabel: 'This device',
                detail: 'Last seen ${ago(device.lastSeenAt)}',
                trailing: controller.isThisDevice(device.id)
                    ? null
                    : NfButton(
                        label: 'Forget',
                        semanticLabel: 'Forget ${device.name}',
                        style: NfButtonStyle.ghost,
                        size: NfButtonSize.small,
                        onPressed: () => unawaited(_forget(device)),
                      ),
              ),
        ],
      ),
    );
  }
}
