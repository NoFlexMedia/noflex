// SPDX-License-Identifier: AGPL-3.0-only
//
// Deleting your own account, which app stores require an app that makes
// accounts to offer. The last administrator's takes the whole server with it,
// so that asks twice, the second time for a phrase typed out.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import 'account_controller.dart';
import 'account_widgets.dart';

/// What erasing the server asks to have typed, as the web asks.
const erasePhrase = 'delete everything';

class DeleteAccountPanel extends StatefulWidget {
  const DeleteAccountPanel({super.key, required this.controller});
  final AccountController controller;

  @override
  State<DeleteAccountPanel> createState() => _DeleteAccountPanelState();
}

class _DeleteAccountPanelState extends State<DeleteAccountPanel> {
  final _asking = signal(false);

  Future<void> _open() async {
    _asking.value = true;
    final terms = await widget.controller.accountDeletion();
    if (!mounted) return;
    _asking.value = false;
    final username = getIt<SessionService>().viewer.value?.username;
    if (terms == null || username == null) return;
    await getIt<NavService>().openSheet<void>(
      (context) => _DeleteAccountSheet(
        controller: widget.controller,
        passwordRequired: terms.passwordRequired,
        erasesServer: terms.erasesServer,
        username: username,
      ),
    );
  }

  @override
  Widget build(BuildContext context) => AccountPanel(
    icon: PhosphorIconsRegular.trash,
    title: 'Delete account',
    detail: 'For good, with everything that’s yours on this server',
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          'Your watch history, devices, playlists and linked services go, '
          'and so does anything you downloaded to this device. Nobody can '
          'bring them back, your administrator included.',
          style: NfType.small,
        ),
        const SizedBox(height: 14),
        Align(
          alignment: Alignment.centerRight,
          child: SignalBuilder(
            builder: (context) => NfButton(
              label: 'Delete account…',
              style: NfButtonStyle.danger,
              size: NfButtonSize.small,
              icon: const Icon(PhosphorIconsRegular.trash),
              busy: _asking.value,
              onPressed: () => unawaited(_open()),
            ),
          ),
        ),
      ],
    ),
  );
}

enum _Step {
  /// An ordinary deletion, asked once.
  confirm,

  /// The first warning that the server goes too.
  warn,

  /// The second, which takes the phrase typed out.
  erase,
}

class _DeleteAccountSheet extends StatefulWidget {
  const _DeleteAccountSheet({
    required this.controller,
    required this.passwordRequired,
    required this.erasesServer,
    required this.username,
  });

  final AccountController controller;
  final bool passwordRequired;
  final bool erasesServer;
  final String username;

  @override
  State<_DeleteAccountSheet> createState() => _DeleteAccountSheetState();
}

class _DeleteAccountSheetState extends State<_DeleteAccountSheet> {
  final _credential = TextEditingController();
  final _phrase = TextEditingController();
  late final _step = signal(widget.erasesServer ? _Step.warn : _Step.confirm);
  final _busy = signal(false);

  /// Became the last administrator between opening this and confirming.
  final _becameLast = signal(false);

  /// The fields' text as it changes, for what the button depends on.
  final _text = signal((credential: '', phrase: ''));

  void _changed(String _) =>
      _text.value = (credential: _credential.text, phrase: _phrase.text);

  @override
  void dispose() {
    _credential.dispose();
    _phrase.dispose();
    super.dispose();
  }

  bool get _ready {
    final text = _text.value;
    if (text.credential.trim().isEmpty) return false;
    return _step.value != _Step.erase || text.phrase.trim() == erasePhrase;
  }

  Future<void> _submit() async {
    if (!_ready || _busy.value) return;
    _busy.value = true;
    final credential = _credential.text;
    final outcome = await widget.controller.deleteAccount(
      password: widget.passwordRequired ? credential : null,
      username: widget.passwordRequired ? null : credential.trim(),
      eraseServer: _step.value == _Step.erase,
    );
    if (!mounted) return;
    _busy.value = false;
    if (outcome == AccountDeletionOutcome.erasesServer) {
      batch(() {
        _becameLast.value = true;
        _step.value = _Step.warn;
      });
    }
  }

  @override
  Widget build(BuildContext context) => Padding(
    // A dialog already lifts itself above the keyboard; a sheet has to ask.
    padding: EdgeInsets.fromLTRB(
      20,
      20,
      20,
      20 +
          (context.windowClass.isCompact
              ? MediaQuery.viewInsetsOf(context).bottom
              : 0),
    ),
    child: SingleChildScrollView(
      child: SignalBuilder(
        builder: (context) => switch (_step.value) {
          _Step.confirm => _confirm(context),
          _Step.warn => _warn(context),
          _Step.erase => _erase(context),
        },
      ),
    ),
  );

  Widget _confirm(BuildContext context) => _Layout(
    title: 'Delete your account?',
    body: [
      Text(
        'This deletes your account on this server for good: your watch '
        'history, ratings and favorites, your devices, playlists and '
        'invites, linked services and servers, and offline copies. Anything '
        'you downloaded to this device goes too. It can’t be undone.',
        style: NfType.small,
      ),
      const SizedBox(height: 14),
      _credentialField(),
    ],
    action: _action('Delete account'),
  );

  Widget _warn(BuildContext context) => _Layout(
    title: 'Deleting your account erases this server',
    body: [
      if (_becameLast.value) ...[
        Text(
          'You’ve become this server’s last administrator since you opened '
          'this.',
          style: nfText(13, weight: 600, color: NfColors.danger),
        ),
        const SizedBox(height: 8),
      ],
      Text(
        'You’re its last administrator, so your account only goes with '
        'everything else on it. Every account is deleted, and everyone is '
        'signed out on every device. Libraries, watch history, settings, '
        'linked servers and services, and everything the server made from '
        'your media (transcodes, optimized versions, artwork and the search '
        'index) are erased.',
        style: NfType.small,
      ),
      const SizedBox(height: 12),
      const _Kept(),
    ],
    action: NfButton(
      label: 'Continue',
      style: NfButtonStyle.danger,
      size: NfButtonSize.small,
      onPressed: () => _step.value = _Step.erase,
    ),
  );

  Widget _erase(BuildContext context) => _Layout(
    title: 'Erase this server?',
    body: [
      Text(
        'Type “$erasePhrase” to confirm. Nobody can undo this.',
        style: NfType.small,
      ),
      const SizedBox(height: 14),
      NfField(
        controller: _phrase,
        label: 'Type “$erasePhrase”',
        autofocus: true,
        autocorrect: false,
        textInputAction: TextInputAction.next,
        onChanged: _changed,
      ),
      const SizedBox(height: 12),
      _credentialField(),
    ],
    action: _action('Erase server'),
  );

  Widget _credentialField() => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      NfField(
        controller: _credential,
        label: widget.passwordRequired ? 'Your password' : 'Your username',
        obscure: widget.passwordRequired,
        autocorrect: false,
        autofillHints: widget.passwordRequired
            ? const [AutofillHints.password]
            : null,
        textInputAction: TextInputAction.done,
        onChanged: _changed,
        onSubmitted: (_) => unawaited(_submit()),
      ),
      if (!widget.passwordRequired) ...[
        const SizedBox(height: 4),
        Text(
          'You sign in through another service, so type your username, '
          '${widget.username}, to confirm.',
          style: NfType.caption,
        ),
      ],
    ],
  );

  Widget _action(String label) => NfButton(
    label: label,
    style: NfButtonStyle.danger,
    size: NfButtonSize.small,
    busy: _busy.value,
    onPressed: _ready ? () => unawaited(_submit()) : null,
  );
}

class _Layout extends StatelessWidget {
  const _Layout({
    required this.title,
    required this.body,
    required this.action,
  });

  final String title;
  final List<Widget> body;
  final Widget action;

  @override
  Widget build(BuildContext context) => Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      Text(title, style: nfText(18, weight: 650)),
      const SizedBox(height: 8),
      ...body,
      const SizedBox(height: 18),
      Row(
        mainAxisAlignment: MainAxisAlignment.end,
        spacing: 8,
        children: [
          NfButton(
            label: 'Cancel',
            style: NfButtonStyle.ghost,
            size: NfButtonSize.small,
            onPressed: () => Navigator.of(context).pop(),
          ),
          action,
        ],
      ),
    ],
  );
}

/// What an erasure leaves, set apart so it isn't lost among what goes.
class _Kept extends StatelessWidget {
  const _Kept();

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.all(12),
    decoration: BoxDecoration(
      color: NfColors.surface2,
      borderRadius: BorderRadius.circular(NfRadii.m),
      border: Border.all(color: NfColors.line),
    ),
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      spacing: 10,
      children: [
        const Icon(
          PhosphorIconsRegular.filmStrip,
          size: 18,
          color: NfColors.text2,
        ),
        Expanded(
          child: Text(
            'Media files on disk and the server’s configuration file are '
            'kept. The server comes back at first-run setup, as if just '
            'installed.',
            style: NfType.small,
          ),
        ),
      ],
    ),
  );
}
