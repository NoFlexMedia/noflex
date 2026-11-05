// SPDX-License-Identifier: AGPL-3.0-only
//
// Everything before the shell: choosing a server, signing in, first-run setup,
// redeeming an invite, and approving this device from a signed-in browser for
// accounts without a password (external sign-in).

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/queries/__generated__/create_device_pin.req.gql.dart';
import '../../graphql/queries/__generated__/poll_device_pin.req.gql.dart';
import '../../services/device_identity.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../../services/tokens.dart';
import '../../theme.dart';
import '../../widgets/brand_mark.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../downloads/leave_account.dart';

/// What someone typed, as an origin: `192.168.1.5:8096` → `http://192.168.1.5:8096`.
/// Null when it can't be one. A bare host is tried as http: home servers
/// rarely have a certificate, and one that does redirects.
String? normalizeOrigin(String input) {
  var value = input.trim();
  if (value.isEmpty) return null;
  value = value
      .replaceFirst(RegExp(r'/+$'), '')
      .replaceFirst(RegExp(r'/graphql$'), '');
  if (!value.contains('://')) value = 'http://$value';
  final uri = Uri.tryParse(value);
  if (uri == null ||
      uri.host.isEmpty ||
      !(uri.isScheme('http') || uri.isScheme('https'))) {
    return null;
  }
  return uri.toString().replaceFirst(RegExp(r'/+$'), '');
}

class _AuthFrame extends StatelessWidget {
  const _AuthFrame({
    required this.title,
    this.subtitle,
    required this.children,
  });

  final String title;
  final String? subtitle;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) => Scaffold(
    body: SafeArea(
      child: Center(
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(24),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 400),
            child: AutofillGroup(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const Align(
                    alignment: Alignment.centerLeft,
                    child: BrandMark(size: 44),
                  ),
                  const SizedBox(height: 22),
                  Text(title, style: NfType.pageTitle),
                  if (subtitle != null) ...[
                    const SizedBox(height: 6),
                    Text(subtitle!, style: NfType.bodyMuted),
                  ],
                  const SizedBox(height: 24),
                  ...children,
                ],
              ),
            ),
          ),
        ),
      ),
    ),
  );
}

class _ErrorLine extends StatelessWidget {
  const _ErrorLine(this.message);
  final String message;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 14),
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        const Icon(
          PhosphorIconsFill.warningCircle,
          size: 18,
          color: NfColors.danger,
        ),
        const SizedBox(width: 8),
        Expanded(
          child: Text(message, style: nfText(14, color: NfColors.danger)),
        ),
      ],
    ),
  );
}

class ConnectScreen extends StatefulWidget {
  const ConnectScreen({super.key});

  @override
  State<ConnectScreen> createState() => _ConnectScreenState();
}

class _ConnectScreenState extends State<ConnectScreen> {
  final _controller = TextEditingController();
  final _error = signal('');
  final _busy = signal(false);

  Future<void> _submit() async {
    final origin = normalizeOrigin(_controller.text);
    if (origin == null) {
      _error.value = 'Enter the address of your NoFlex server.';
      return;
    }
    batch(() {
      _busy.value = true;
      _error.value = '';
    });
    try {
      await getIt<GraphqlService>().probe(origin);
      await getIt<SessionService>().connect(origin);
    } on Exception catch (cause) {
      _error.value = cause.toString();
    } finally {
      _busy.value = false;
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => _AuthFrame(
    title: 'Connect to your server',
    subtitle:
        'NoFlex is self-hosted, so this app needs to know where yours lives.',
    children: [
      SignalBuilder(
        builder: (_) => _error.value.isEmpty
            ? const SizedBox.shrink()
            : _ErrorLine(_error.value),
      ),
      NfField(
        controller: _controller,
        label: 'Server address',
        hint: '192.168.1.20:8096 or media.example.com',
        keyboardType: TextInputType.url,
        textInputAction: TextInputAction.go,
        autocorrect: false,
        autofocus: true,
        onSubmitted: (_) => _submit(),
      ),
      const SizedBox(height: 16),
      SignalBuilder(
        builder: (_) => NfButton(
          label: 'Continue',
          style: NfButtonStyle.primary,
          size: NfButtonSize.large,
          expand: true,
          busy: _busy.value,
          onPressed: _submit,
        ),
      ),
    ],
  );
}

enum _AuthMode { signIn, invite, pin }

class SignInScreen extends StatefulWidget {
  const SignInScreen({super.key});

  @override
  State<SignInScreen> createState() => _SignInScreenState();
}

class _SignInScreenState extends State<SignInScreen> {
  final _username = TextEditingController();
  final _password = TextEditingController();
  final _invite = TextEditingController();
  final _mode = signal(_AuthMode.signIn);
  final _busy = signal(false);
  final _error = signal('');
  final _pin = signal<String?>(null);
  Timer? _poll;

  SessionService get _session => getIt<SessionService>();

  Future<void> _run(Future<void> Function() action) async {
    batch(() {
      _busy.value = true;
      _error.value = '';
    });
    try {
      await action();
    } on Exception catch (cause) {
      _error.value = cause.toString();
    } finally {
      _busy.value = false;
    }
  }

  void _submit() {
    final username = _username.text.trim();
    final password = _password.text;
    if (_mode.value == _AuthMode.invite) {
      unawaited(
        _run(
          () => _session.redeemInvite(
            _inviteToken(_invite.text),
            username,
            password,
          ),
        ),
      );
    } else {
      unawaited(_run(() => _session.login(username, password)));
    }
  }

  /// A pasted invite link works as well as its token.
  static String _inviteToken(String input) {
    final trimmed = input.trim();
    return Uri.tryParse(trimmed)?.queryParameters['token'] ?? trimmed;
  }

  Future<void> _startPin() async {
    _mode.value = _AuthMode.pin;
    final graphql = getIt<GraphqlService>();
    final identity = getIt<DeviceIdentity>();
    await _run(() async {
      final data = await graphql.run(
        GCreateDevicePinReq(
          (b) => b.vars
            ..client = identity.clientName
            ..device = identity.name,
        ),
      );
      final pin = data.createDevicePin.pin;
      _pin.value = pin;
      _poll?.cancel();
      _poll = Timer.periodic(const Duration(seconds: 3), (_) => _checkPin(pin));
    });
  }

  Future<void> _checkPin(String pin) async {
    try {
      final data = await getIt<GraphqlService>().run(
        GPollDevicePinReq((b) => b.vars.pin = pin),
      );
      final result = data.pollDevicePin;
      if (result == null) return;
      _poll?.cancel();
      await _session.signIn(access: result.token, refresh: result.refreshToken);
    } on ServerUnreachable {
      // Keep polling; the PIN outlives a blip.
    } on ApiError catch (cause) {
      _poll?.cancel();
      batch(() {
        _pin.value = null;
        _error.value = cause.message;
      });
    }
  }

  @override
  void dispose() {
    _poll?.cancel();
    _username.dispose();
    _password.dispose();
    _invite.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (_) {
      final mode = _mode.value;
      final server =
          _session.serverName.value ?? getIt<TokenStore>().origin.value ?? '';
      final message = _error.value.isNotEmpty
          ? _error.value
          : _session.authError.value;
      return _AuthFrame(
        title: switch (mode) {
          _AuthMode.signIn => 'Sign in',
          _AuthMode.invite => 'Accept your invite',
          _AuthMode.pin => 'Approve this device',
        },
        subtitle: server,
        children: [
          if (message.isNotEmpty) _ErrorLine(message),
          if (mode == _AuthMode.pin) ...[
            Text(
              'On a device that’s already signed in, open Account → Devices and enter this code.',
              style: NfType.bodyMuted,
            ),
            const SizedBox(height: 18),
            Center(
              child: _pin.value == null
                  ? const CircularProgressIndicator()
                  : SelectableText(
                      _pin.value!,
                      style: nfText(
                        40,
                        weight: 700,
                        letterSpacing: 6,
                        features: NfType.tabular,
                      ),
                    ),
            ),
            const SizedBox(height: 18),
            NfButton(
              label: 'Back to sign in',
              style: NfButtonStyle.ghost,
              expand: true,
              onPressed: () {
                _poll?.cancel();
                _mode.value = _AuthMode.signIn;
              },
            ),
          ] else ...[
            if (mode == _AuthMode.invite) ...[
              NfField(
                controller: _invite,
                label: 'Invite link or code',
                autocorrect: false,
                textInputAction: TextInputAction.next,
              ),
              const SizedBox(height: 12),
            ],
            NfField(
              controller: _username,
              label: mode == _AuthMode.invite
                  ? 'Choose a username'
                  : 'Username',
              autocorrect: false,
              textInputAction: TextInputAction.next,
              autofillHints: const [AutofillHints.username],
            ),
            const SizedBox(height: 12),
            NfField(
              controller: _password,
              label: mode == _AuthMode.invite
                  ? 'Choose a password'
                  : 'Password',
              obscure: true,
              textInputAction: TextInputAction.go,
              autofillHints: const [AutofillHints.password],
              onSubmitted: (_) => _submit(),
            ),
            const SizedBox(height: 16),
            NfButton(
              label: mode == _AuthMode.invite ? 'Create account' : 'Sign in',
              style: NfButtonStyle.primary,
              size: NfButtonSize.large,
              expand: true,
              busy: _busy.value,
              onPressed: _submit,
            ),
            const SizedBox(height: 18),
            Wrap(
              alignment: WrapAlignment.center,
              spacing: 6,
              children: [
                TextButton(
                  onPressed: () => _mode.value = mode == _AuthMode.invite
                      ? _AuthMode.signIn
                      : _AuthMode.invite,
                  child: Text(
                    mode == _AuthMode.invite
                        ? 'I have an account'
                        : 'I have an invite',
                    style: nfText(13, weight: 600, color: NfColors.text2),
                  ),
                ),
                TextButton(
                  onPressed: _startPin,
                  child: Text(
                    'Sign in with a code',
                    style: nfText(13, weight: 600, color: NfColors.text2),
                  ),
                ),
                TextButton(
                  onPressed: () => unawaited(changeServer()),
                  child: Text(
                    'Change server',
                    style: nfText(13, weight: 600, color: NfColors.text2),
                  ),
                ),
              ],
            ),
          ],
        ],
      );
    },
  );
}

class SetupScreen extends StatefulWidget {
  const SetupScreen({super.key});

  @override
  State<SetupScreen> createState() => _SetupScreenState();
}

class _SetupScreenState extends State<SetupScreen> {
  final _username = TextEditingController();
  final _password = TextEditingController();
  final _busy = signal(false);
  final _error = signal('');

  Future<void> _submit() async {
    batch(() {
      _busy.value = true;
      _error.value = '';
    });
    try {
      await getIt<SessionService>().completeSetup(
        _username.text.trim(),
        _password.text,
      );
    } on Exception catch (cause) {
      _error.value = cause.toString();
    } finally {
      _busy.value = false;
    }
  }

  @override
  void dispose() {
    _username.dispose();
    _password.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => _AuthFrame(
    title: 'Welcome to NoFlex',
    subtitle: 'This server has no accounts yet. Create the administrator.',
    children: [
      SignalBuilder(
        builder: (_) => _error.value.isEmpty
            ? const SizedBox.shrink()
            : _ErrorLine(_error.value),
      ),
      NfField(
        controller: _username,
        label: 'Username',
        autocorrect: false,
        textInputAction: TextInputAction.next,
      ),
      const SizedBox(height: 12),
      NfField(
        controller: _password,
        label: 'Password',
        obscure: true,
        textInputAction: TextInputAction.go,
        onSubmitted: (_) => _submit(),
      ),
      const SizedBox(height: 16),
      SignalBuilder(
        builder: (_) => NfButton(
          label: 'Create account',
          style: NfButtonStyle.primary,
          size: NfButtonSize.large,
          expand: true,
          busy: _busy.value,
          onPressed: _submit,
        ),
      ),
    ],
  );
}
