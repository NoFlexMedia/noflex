// SPDX-License-Identifier: AGPL-3.0-only
//
// "Who's watching?": everyone signed in on this device, to choose between
// without signing in again. Only people who have signed in here appear. The
// server never lists its users to a device that hasn't.
//
// A profile with a PIN asks for it, and the server is what checks it: the
// session it opens was locked there when its owner stepped away.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
import '../../theme.dart';
import '../../widgets/brand_mark.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_press.dart';
import '../../widgets/user_avatar.dart';
import '../downloads/leave_account.dart';
import 'connect_screen.dart';

const _tileWidth = 132.0;
const _avatarSize = 96.0;

class ProfilesScreen extends StatefulWidget {
  const ProfilesScreen({super.key});

  @override
  State<ProfilesScreen> createState() => _ProfilesScreenState();
}

class _ProfilesScreenState extends State<ProfilesScreen> {
  /// Whose profile is opening, so their tile can say so.
  final _entering = signal<String?>(null);

  SessionService get _session => getIt<SessionService>();

  Future<void> _choose(StoredAccount account) async {
    if (_entering.value != null) return;
    if (account.needsSignIn) {
      _session.addProfile();
      return;
    }
    if (account.hasPin) {
      if (!_session.reachable.value) {
        getIt<ToastService>().show(
          'A PIN needs the server to check it. Try again once it’s back.',
          kind: ToastKind.error,
        );
        return;
      }
      await getIt<NavService>().openSheet<void>(
        (context) => _PinSheet(account: account),
      );
      return;
    }
    _entering.value = account.userId;
    await getIt<ToastService>().attempt(
      () => _session.chooseProfile(account.userId),
    );
    if (mounted) _entering.value = null;
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    body: SafeArea(
      child: Center(
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(24),
          child: SignalBuilder(
            builder: (context) {
              final tokens = getIt<TokenStore>();
              final server =
                  _session.serverName.value ?? tokens.origin.value ?? '';
              final message = _session.authError.value;
              final offline = !_session.reachable.value;
              return Column(
                children: [
                  const BrandMark(size: 44),
                  const SizedBox(height: 22),
                  Text(
                    'Who’s watching?',
                    textAlign: TextAlign.center,
                    style: NfType.pageTitle,
                  ),
                  const SizedBox(height: 6),
                  Text(
                    server,
                    textAlign: TextAlign.center,
                    style: NfType.bodyMuted,
                  ),
                  const SizedBox(height: 24),
                  if (message.isNotEmpty)
                    ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 400),
                      child: AuthErrorLine(message),
                    ),
                  if (offline)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 18),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          const Icon(
                            PhosphorIconsRegular.cloudSlash,
                            size: 18,
                            color: NfColors.warning,
                          ),
                          const SizedBox(width: 8),
                          Flexible(
                            child: Text(
                              'Can’t reach your server. Downloads still play.',
                              style: nfText(13.5),
                            ),
                          ),
                          TextButton(
                            onPressed: () => unawaited(_session.start()),
                            child: Text(
                              'Retry',
                              style: nfText(
                                13.5,
                                weight: 600,
                                color: NfColors.accent,
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  Wrap(
                    alignment: WrapAlignment.center,
                    spacing: context.windowClass.isCompact ? 12 : 22,
                    runSpacing: 22,
                    children: [
                      for (final account in tokens.accounts.value)
                        _ProfileTile(
                          account: account,
                          entering: _entering.value == account.userId,
                          dimmed: offline && account.hasPin,
                          onTap: () => unawaited(_choose(account)),
                        ),
                      // Signing in needs the server, as a PIN does.
                      if (!offline)
                        _Tile(
                          label: 'Add someone',
                          onTap: _session.addProfile,
                          picture: const _AddPicture(),
                        ),
                    ],
                  ),
                  const SizedBox(height: 28),
                  TextButton(
                    onPressed: () => unawaited(changeServer()),
                    child: Text(
                      'Change server',
                      style: nfText(13, weight: 600, color: NfColors.text2),
                    ),
                  ),
                ],
              );
            },
          ),
        ),
      ),
    ),
  );
}

class _ProfileTile extends StatelessWidget {
  const _ProfileTile({
    required this.account,
    required this.entering,
    required this.dimmed,
    required this.onTap,
  });

  final StoredAccount account;
  final bool entering;
  final bool dimmed;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => _Tile(
    label: account.name,
    semanticLabel: account.needsSignIn
        ? '${account.name}, sign in again'
        : account.hasPin
        ? '${account.name}, locked with a PIN'
        : account.name,
    detail: account.needsSignIn ? 'Sign in again' : null,
    dimmed: dimmed || account.needsSignIn,
    onTap: onTap,
    picture: Stack(
      clipBehavior: Clip.none,
      children: [
        UserAvatar(
          name: account.name,
          avatarUrl: account.avatarUrl,
          size: _avatarSize,
        ),
        if (entering)
          const Positioned.fill(
            child: DecoratedBox(
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: NfColors.bgOverlay,
              ),
              child: Center(
                child: SizedBox.square(
                  dimension: 26,
                  child: CircularProgressIndicator(strokeWidth: 2.5),
                ),
              ),
            ),
          ),
        if (account.hasPin && !account.needsSignIn)
          const Positioned(
            right: -2,
            bottom: -2,
            child: _Badge(PhosphorIconsFill.lockSimple),
          ),
      ],
    ),
  );
}

class _Badge extends StatelessWidget {
  const _Badge(this.icon);
  final IconData icon;

  @override
  Widget build(BuildContext context) => Container(
    width: 30,
    height: 30,
    decoration: BoxDecoration(
      shape: BoxShape.circle,
      color: NfColors.bgRaised,
      border: Border.all(color: NfColors.lineStrong),
    ),
    child: Icon(icon, size: 15, color: NfColors.text),
  );
}

class _AddPicture extends StatelessWidget {
  const _AddPicture();

  @override
  Widget build(BuildContext context) => Container(
    width: _avatarSize,
    height: _avatarSize,
    decoration: BoxDecoration(
      shape: BoxShape.circle,
      color: NfColors.surface2,
      border: Border.all(color: NfColors.lineStrong),
    ),
    child: const Icon(
      PhosphorIconsRegular.plus,
      size: 30,
      color: NfColors.text2,
    ),
  );
}

/// A picture over a name, sinking under a press as a card does.
class _Tile extends StatelessWidget {
  const _Tile({
    required this.label,
    required this.picture,
    required this.onTap,
    this.semanticLabel,
    this.detail,
    this.dimmed = false,
  });

  final String label;
  final String? semanticLabel;
  final String? detail;
  final Widget picture;
  final bool dimmed;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: semanticLabel ?? label,
    excludeSemantics: true,
    child: NfPressable(
      scale: 0.95,
      builder: (context, onPress) => Material(
        color: Colors.transparent,
        borderRadius: BorderRadius.circular(NfRadii.l),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          onHighlightChanged: onPress,
          child: SizedBox(
            width: _tileWidth,
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 14, horizontal: 8),
              child: Column(
                children: [
                  Opacity(opacity: dimmed ? 0.5 : 1, child: picture),
                  const SizedBox(height: 12),
                  Text(
                    label,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    textAlign: TextAlign.center,
                    style: nfText(
                      15,
                      weight: 600,
                      color: dimmed ? NfColors.text2 : NfColors.text,
                    ),
                  ),
                  if (detail != null) ...[
                    const SizedBox(height: 2),
                    Text(
                      detail!,
                      textAlign: TextAlign.center,
                      style: NfType.caption,
                    ),
                  ],
                ],
              ),
            ),
          ),
        ),
      ),
    ),
  );
}

class _PinSheet extends StatefulWidget {
  const _PinSheet({required this.account});
  final StoredAccount account;

  @override
  State<_PinSheet> createState() => _PinSheetState();
}

class _PinSheetState extends State<_PinSheet> {
  final _pin = TextEditingController();
  final _busy = signal(false);
  final _error = signal('');
  final _ready = signal(false);

  @override
  void dispose() {
    _pin.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    if (!_ready.value || _busy.value) return;
    batch(() {
      _busy.value = true;
      _error.value = '';
    });
    try {
      await getIt<SessionService>().chooseProfile(
        widget.account.userId,
        pin: _pin.text,
      );
      if (mounted) Navigator.of(context).pop();
    } on WrongPin catch (wrong) {
      _pin.clear();
      batch(() {
        _ready.value = false;
        _error.value = wrong.toString();
      });
    } on ServerUnreachable catch (cause) {
      _error.value = cause.message;
    } on Exception catch (cause) {
      // Out of tries, or the session is gone: the tile says what's next, and
      // there's nothing more to do here.
      if (mounted) Navigator.of(context).pop();
      getIt<ToastService>().show(cause.toString(), kind: ToastKind.error);
    } finally {
      _busy.value = false;
    }
  }

  @override
  Widget build(BuildContext context) => Padding(
    // A dialog already lifts itself above the keyboard; a sheet has to ask.
    padding: EdgeInsets.fromLTRB(
      22,
      22,
      22,
      22 +
          (context.windowClass.isCompact
              ? MediaQuery.viewInsetsOf(context).bottom
              : 0),
    ),
    child: SignalBuilder(
      builder: (context) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            spacing: 12,
            children: [
              UserAvatar(
                name: widget.account.name,
                avatarUrl: widget.account.avatarUrl,
                size: 44,
              ),
              Expanded(
                child: Text(
                  'Enter ${widget.account.name}’s PIN',
                  style: NfType.sectionTitle,
                ),
              ),
            ],
          ),
          const SizedBox(height: 16),
          NfField(
            controller: _pin,
            obscure: true,
            autofocus: true,
            autocorrect: false,
            keyboardType: TextInputType.number,
            textInputAction: TextInputAction.done,
            inputFormatters: [
              FilteringTextInputFormatter.digitsOnly,
              LengthLimitingTextInputFormatter(6),
            ],
            onChanged: (value) => _ready.value = value.length >= 4,
            onSubmitted: (_) => unawaited(_submit()),
          ),
          if (_error.value.isNotEmpty) ...[
            const SizedBox(height: 8),
            Text(_error.value, style: nfText(13, color: NfColors.danger)),
          ],
          const SizedBox(height: 18),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            spacing: 10,
            children: [
              NfButton(
                label: 'Cancel',
                style: NfButtonStyle.ghost,
                onPressed: () => Navigator.of(context).pop(),
              ),
              NfButton(
                label: 'Unlock',
                style: NfButtonStyle.primary,
                busy: _busy.value,
                onPressed: _ready.value ? () => unawaited(_submit()) : null,
              ),
            ],
          ),
        ],
      ),
    ),
  );
}
