// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/queries/fragments/__generated__/linked_server_fields.data.gql.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../auth/connect_screen.dart';
import 'account_controller.dart';
import 'account_widgets.dart';
import 'server_link.dart';

class ServersSection extends SignalWidget {
  const ServersSection({super.key, required this.controller});
  final AccountController controller;

  Future<void> _link([GLinkedServerFields? existing]) async {
    await getIt<NavService>().openSheet<void>(
      (_) => _LinkSheet(controller: controller, existing: existing),
    );
  }

  Future<void> _remove(GLinkedServerFields server) async {
    final confirmed = await confirmAction(
      title: 'Remove ${server.name}?',
      message: 'Its titles leave your home screen and search. To fully cut access, also revoke the key from your account on that server.',
      confirmLabel: 'Remove server',
      danger: true,
    );
    if (confirmed) await controller.removeServer(server.id);
  }

  @override
  Widget build(BuildContext context) {
    final servers = getIt<SessionService>().linkedServers.value;
    return AccountColumn(
      children: [
        AccountPanel(
          icon: PhosphorIconsRegular.hardDrives,
          title: 'Other servers',
          detail: 'Browse friends’ NoFlex servers alongside this one. Your device talks to them directly.',
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (servers.isEmpty)
                Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: Text(
                    'Only this server so far. Add another you have an account on, or ask a friend for an invite to theirs.',
                    style: NfType.small,
                  ),
                )
              else
                for (final server in servers)
                  _ServerRow(
                    server: server,
                    onSignIn: () => unawaited(_link(server)),
                    onRemove: () => unawaited(_remove(server)),
                    onSync: (enabled) => unawaited(
                      controller.setSync(server.id, enabled: enabled),
                    ),
                  ),
              const SizedBox(height: 4),
              Align(
                alignment: Alignment.centerLeft,
                child: NfButton(
                  label: 'Add a server',
                  size: NfButtonSize.small,
                  icon: const Icon(PhosphorIconsRegular.plus),
                  onPressed: () => unawaited(_link()),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

class _ServerRow extends StatelessWidget {
  const _ServerRow({
    required this.server,
    required this.onSignIn,
    required this.onRemove,
    required this.onSync,
  });

  final GLinkedServerFields server;
  final VoidCallback onSignIn;
  final VoidCallback onRemove;
  final ValueChanged<bool> onSync;

  @override
  Widget build(BuildContext context) {
    final connected = server.apiKey.isNotEmpty;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        AccountRow(
          icon: PhosphorIconsRegular.hardDrives,
          title: server.name,
          detail: connected
              ? server.origin
              : '${server.origin} · not signed in',
          trailing: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (!connected)
                NfButton(
                  label: 'Sign in',
                  size: NfButtonSize.small,
                  onPressed: onSignIn,
                ),
              NfIconButton(
                icon: PhosphorIconsRegular.trash,
                label: 'Remove ${server.name}',
                size: 40,
                iconSize: 17,
                color: NfColors.text3,
                onPressed: onRemove,
              ),
            ],
          ),
        ),
        if (connected)
          Padding(
            padding: const EdgeInsets.only(left: 50, bottom: 8),
            child: AccountToggle(
              title: 'Sync what I watch',
              detail:
                  'Keeps your place on both servers for titles they share. ${server.name}’s administrators can see what you’ve watched there, including what syncs from here.',
              value: server.syncEnabled,
              onChanged: onSync,
            ),
          ),
      ],
    );
  }
}

class _LinkSheet extends StatefulWidget {
  const _LinkSheet({required this.controller, this.existing});

  final AccountController controller;

  /// One saved without a sign-in, from the web; only the account's missing.
  final GLinkedServerFields? existing;

  @override
  State<_LinkSheet> createState() => _LinkSheetState();
}

class _LinkSheetState extends State<_LinkSheet> {
  final _address = TextEditingController();
  final _username = TextEditingController();
  final _password = TextEditingController();
  final _busy = signal(false);
  final _ready = signal(false);

  void _changed(String _) => _ready.value =
      (widget.existing != null || _address.text.trim().isNotEmpty) &&
      _username.text.isNotEmpty &&
      _password.text.isNotEmpty;

  @override
  void dispose() {
    _address.dispose();
    _username.dispose();
    _password.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    if (_busy.value) return;
    final toasts = getIt<ToastService>();
    final origin = widget.existing?.origin ?? normalizeOrigin(_address.text);
    if (origin == null) {
      toasts.show(
        'Enter a server address like https://media.example.com',
        kind: ToastKind.error,
      );
      return;
    }
    _busy.value = true;
    var linked = false;
    await toasts.attempt(() async {
      final name = await linkServer(
        origin: origin,
        username: _username.text,
        password: _password.text,
      );
      if (name == null) return;
      await widget.controller.reloadLinkedServers();
      linked = true;
      toasts.show('Connected to $name', kind: ToastKind.success);
    });
    if (!mounted) return;
    _busy.value = false;
    if (linked) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) {
    final existing = widget.existing;
    return Padding(
      // A dialog already lifts itself above the keyboard; a sheet has to ask.
      padding: EdgeInsets.only(
        bottom: context.windowClass.isCompact
            ? MediaQuery.viewInsetsOf(context).bottom
            : 0,
      ),
      child: SafeArea(
        top: false,
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(22),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                existing == null
                    ? 'Add a server'
                    : 'Sign in to ${existing.name}',
                style: NfType.sectionTitle,
              ),
              const SizedBox(height: 16),
              if (existing == null) ...[
                NfField(
                  controller: _address,
                  label: 'Server address',
                  hint: 'https://media.example.com',
                  autofocus: true,
                  autocorrect: false,
                  keyboardType: TextInputType.url,
                  textInputAction: TextInputAction.next,
                  onChanged: _changed,
                ),
                const SizedBox(height: 12),
              ],
              NfField(
                controller: _username,
                label: 'Username there',
                autofocus: existing != null,
                autocorrect: false,
                textInputAction: TextInputAction.next,
                onChanged: _changed,
              ),
              const SizedBox(height: 12),
              NfField(
                controller: _password,
                label: 'Password',
                obscure: true,
                autocorrect: false,
                textInputAction: TextInputAction.done,
                onChanged: _changed,
                onSubmitted: (_) {
                  if (_ready.value) unawaited(_submit());
                },
              ),
              const SizedBox(height: 4),
              Text(
                'Used once to create a key, and never stored. You can revoke the key from your account there.',
                style: NfType.caption,
              ),
              const SizedBox(height: 20),
              Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  NfButton(
                    label: 'Cancel',
                    style: NfButtonStyle.ghost,
                    onPressed: () => Navigator.of(context).pop(),
                  ),
                  const SizedBox(width: 10),
                  SignalBuilder(
                    builder: (context) => NfButton(
                      label: 'Connect',
                      style: NfButtonStyle.primary,
                      busy: _busy.value,
                      onPressed: _ready.value
                          ? () => unawaited(_submit())
                          : null,
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}
