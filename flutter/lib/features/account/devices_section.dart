// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
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

const _prepareQualities = [
  (null, 'Best it can play'),
  (20000000, 'Up to 20 Mbps'),
  (8000000, 'Up to 8 Mbps'),
  (4000000, 'Up to 4 Mbps'),
];

/// What someone comes here to do first, then the lists, which can run long. Wider
/// than the other sections, as these rows hold several controls each.
class DevicesSection extends StatelessWidget {
  const DevicesSection({super.key, required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      LayoutBuilder(
        builder: (context, constraints) {
          final keys = _ApiKeysPanel(controller: controller);
          final link = _LinkDevicePanel(controller: controller);
          if (constraints.maxWidth < 680) {
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [keys, const SizedBox(height: 14), link],
            );
          }
          return Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(child: keys),
              const SizedBox(width: 14),
              Expanded(child: link),
            ],
          );
        },
      ),
      const SizedBox(height: 14),
      _SmartTranscodingPanel(controller: controller),
      const SizedBox(height: 14),
      _SessionsPanel(controller: controller),
    ],
  );
}

class _ApiKeysPanel extends StatefulWidget {
  const _ApiKeysPanel({required this.controller});
  final AccountController controller;

  @override
  State<_ApiKeysPanel> createState() => _ApiKeysPanelState();
}

class _ApiKeysPanelState extends State<_ApiKeysPanel> {
  final _name = TextEditingController();
  final _hasName = signal(false);
  final _busy = signal(false);

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  Future<void> _create() async {
    final name = _name.text.trim();
    if (name.isEmpty || _busy.value) return;
    _busy.value = true;
    final key = await widget.controller.createApiKey(name);
    if (!mounted) return;
    _busy.value = false;
    if (key == null) return;
    _name.clear();
    _hasName.value = false;
    unawaited(_showKey(name, key));
  }

  /// There's no seeing it again, so this stays up until they've dealt with it.
  Future<void> _showKey(String name, String key) async {
    final copy = await confirmAction(
      title: 'Your new API key',
      message:
          'Copy it now. NoFlex stores only a hash, so this is the only time it’s shown.\n\n$name\n$key',
      confirmLabel: 'Copy key',
      cancelLabel: 'Done',
    );
    if (!copy) return;
    await Clipboard.setData(ClipboardData(text: key));
    getIt<ToastService>().show('API key copied', kind: ToastKind.success);
  }

  Future<void> _revoke(AccountApiKey key) async {
    final confirmed = await confirmAction(
      title: 'Revoke “${key.name}”?',
      message: 'Anything using this key loses access straight away.',
      confirmLabel: 'Revoke key',
      danger: true,
    );
    if (confirmed) await widget.controller.revokeApiKey(key.id);
  }

  @override
  Widget build(BuildContext context) => AccountPanel(
    icon: PhosphorIconsRegular.key,
    title: 'API keys',
    detail: 'For scripts, automations and trusted apps',
    child: SignalBuilder(
      builder: (context) {
        final keys = widget.controller.apiKeys.value;
        final revoked = widget.controller.revokedApiKeys.value;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (keys.isEmpty)
              Padding(
                padding: const EdgeInsets.only(bottom: 10),
                child: Text(
                  revoked > 0
                      ? 'No active keys · $revoked revoked.'
                      : 'No active keys.',
                  style: NfType.small,
                ),
              ),
            for (final key in keys)
              AccountRow(
                title: key.name,
                detail: 'Created ${formatDate(key.createdAt)}',
                trailing: NfButton(
                  label: 'Revoke',
                  semanticLabel: 'Revoke ${key.name}',
                  style: NfButtonStyle.ghost,
                  size: NfButtonSize.small,
                  onPressed: () => unawaited(_revoke(key)),
                ),
              ),
            const SizedBox(height: 6),
            Row(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                Expanded(
                  child: NfField(
                    controller: _name,
                    label: 'New key name',
                    hint: 'Home Assistant',
                    textInputAction: TextInputAction.done,
                    onChanged: (value) =>
                        _hasName.value = value.trim().isNotEmpty,
                    onSubmitted: (_) => unawaited(_create()),
                  ),
                ),
                const SizedBox(width: 10),
                NfButton(
                  label: 'Create',
                  icon: const Icon(PhosphorIconsRegular.plus),
                  busy: _busy.value,
                  onPressed: _hasName.value ? () => unawaited(_create()) : null,
                ),
              ],
            ),
          ],
        );
      },
    ),
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
  final _generated = signal<String?>(null);
  final _approved = signal(false);
  final _generating = signal(false);
  Timer? _poll;

  @override
  void dispose() {
    _poll?.cancel();
    _pin.dispose();
    super.dispose();
  }

  Future<void> _generate() async {
    if (_generating.value) return;
    _generating.value = true;
    final pin = await widget.controller.generatePin();
    if (!mounted) return;
    _generating.value = false;
    if (pin == null) return;
    batch(() {
      _generated.value = pin;
      _approved.value = false;
    });
    _poll?.cancel();
    _poll = Timer.periodic(const Duration(seconds: 3), (_) => _check(pin));
  }

  Future<void> _check(String pin) async {
    try {
      final approved = await widget.controller.pinApproved(pin);
      if (!mounted || !approved) return;
      _poll?.cancel();
      _approved.value = true;
    } on ServerUnreachable {
      // Keep asking; the code outlives a blip.
    } on ApiError {
      // Expired.
      _dismiss();
    }
  }

  void _dismiss() {
    _poll?.cancel();
    batch(() {
      _generated.value = null;
      _approved.value = false;
    });
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
    detail: 'Approve the code shown on a TV or another screen',
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
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
        const SizedBox(height: 12),
        const Divider(height: 1, color: NfColors.line),
        const SizedBox(height: 12),
        SignalBuilder(
          builder: (context) {
            final generated = _generated.value;
            if (generated == null) {
              return Wrap(
                spacing: 12,
                runSpacing: 8,
                crossAxisAlignment: WrapCrossAlignment.center,
                alignment: WrapAlignment.spaceBetween,
                children: [
                  Text(
                    'Need a code for this device instead?',
                    style: NfType.small,
                  ),
                  NfButton(
                    label: 'Generate a PIN',
                    size: NfButtonSize.small,
                    busy: _generating.value,
                    onPressed: () => unawaited(_generate()),
                  ),
                ],
              );
            }
            final approved = _approved.value;
            return Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  approved ? 'APPROVED' : 'ENTER ON A SIGNED-IN DEVICE',
                  style: NfType.eyebrow,
                ),
                const SizedBox(height: 6),
                SelectableText(
                  generated,
                  style: nfText(
                    26,
                    weight: 700,
                    color: approved ? NfColors.success : NfColors.text,
                  ).copyWith(letterSpacing: 4),
                ),
                const SizedBox(height: 4),
                Text(
                  approved
                      ? 'This PIN has been used.'
                      : 'Waiting for approval.',
                  style: NfType.small,
                ),
                const SizedBox(height: 6),
                NfButton(
                  label: 'Dismiss',
                  style: NfButtonStyle.ghost,
                  size: NfButtonSize.small,
                  onPressed: _dismiss,
                ),
              ],
            );
          },
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

class _SmartTranscodingPanel extends SignalWidget {
  const _SmartTranscodingPanel({required this.controller});
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

  Future<void> _chooseQuality(AccountDevice device) async {
    const best = -1;
    final choice = await pickOption<int>(
      title: 'Quality limit for ${device.name}',
      selected: device.maxBitrateBps ?? best,
      options: [
        for (final (value, label) in _prepareQualities)
          NfOption(value ?? best, label),
      ],
    );
    if (choice == null) return;
    await controller.setQualityLimit(device, choice == best ? null : choice);
  }

  Widget _smartRow(AccountDevice device) {
    final name = Row(
      children: [
        Switch(
          value: device.smartTranscode,
          onChanged: (enabled) =>
              unawaited(controller.setSmartTranscode(device, enabled)),
        ),
        const SizedBox(width: 10),
        Expanded(
          child: AccountRow(
            title: device.name,
            isCurrent: controller.isThisDevice(device.id),
            currentLabel: 'This device',
            detail: 'Last seen ${ago(device.lastSeenAt)}',
          ),
        ),
      ],
    );
    final actions = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        NfButton(
          label: _prepareQualities
              .firstWhere(
                (quality) => quality.$1 == device.maxBitrateBps,
                orElse: () => _prepareQualities.first,
              )
              .$2,
          semanticLabel: 'Quality limit for ${device.name}',
          style: NfButtonStyle.ghost,
          size: NfButtonSize.small,
          icon: const Icon(PhosphorIconsRegular.caretDown),
          onPressed: () => unawaited(_chooseQuality(device)),
        ),
        if (!controller.isThisDevice(device.id)) ...[
          const SizedBox(width: 6),
          NfButton(
            label: 'Forget',
            semanticLabel: 'Forget ${device.name}',
            style: NfButtonStyle.ghost,
            size: NfButtonSize.small,
            onPressed: () => unawaited(_forget(device)),
          ),
        ],
      ],
    );
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: LayoutBuilder(
        builder: (context, constraints) => constraints.maxWidth < 560
            ? Column(
                crossAxisAlignment: CrossAxisAlignment.end,
                children: [name, actions],
              )
            : Row(
                children: [
                  Expanded(child: name),
                  const SizedBox(width: 10),
                  actions,
                ],
              ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final devices = controller.devices.value;
    return AccountPanel(
      icon: PhosphorIconsRegular.devices,
      title: 'Smart transcoding',
      detail: 'The server can prepare what you’re likely to watch next while it’s idle, so playback starts without a live transcode',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (devices.isEmpty)
            Text(
              'This device registers itself once it has measured what it can play.',
              style: NfType.small,
            )
          else
            for (final device in devices) _smartRow(device),
        ],
      ),
    );
  }
}
