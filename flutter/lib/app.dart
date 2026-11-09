// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:signals_flutter/signals_flutter.dart';

import 'di.dart';
import 'features/auth/connect_screen.dart';
import 'features/auth/profiles_screen.dart';
import 'features/downloads/downloads_screen.dart';
import 'features/shell/app_shell.dart';
import 'services/nav_service.dart';
import 'services/session.dart';
import 'theme.dart';
import 'widgets/brand_mark.dart';
import 'widgets/nf_overlays.dart';
import 'widgets/nf_state.dart';

class NoFlexApp extends StatelessWidget {
  const NoFlexApp({super.key});

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'NoFlex',
    debugShowCheckedModeBanner: false,
    theme: buildNoFlexTheme(),
    navigatorKey: rootNavigatorKey,
    builder: (context, child) => Stack(children: [?child, const NfToastHost()]),
    home: const _AppFrame(),
  );
}

/// The root stage machine. Flipping the session's phase is all it takes to
/// move between them; nothing navigates manually.
class _AppFrame extends SignalWidget {
  const _AppFrame();

  @override
  Widget build(BuildContext context) {
    final session = getIt<SessionService>();
    return AnimatedSwitcher(
      duration: NfMotion.base,
      child: KeyedSubtree(
        key: ValueKey(session.phase.value),
        child: switch (session.phase.value) {
          AppPhase.boot => const _Boot(),
          AppPhase.connect => const ConnectScreen(),
          AppPhase.error => Scaffold(
            body: NfState.error(
              title: 'NoFlex couldn’t start',
              message: session.error.value,
              retry: session.start,
            ),
          ),
          AppPhase.setup => const SetupScreen(),
          AppPhase.profiles => const ProfilesScreen(),
          AppPhase.auth => const SignInScreen(),
          AppPhase.app => const AppShell(),
          AppPhase.offline => const OfflineShell(),
        },
      ),
    );
  }
}

class _Boot extends StatelessWidget {
  const _Boot();

  @override
  Widget build(BuildContext context) =>
      const Scaffold(body: Center(child: BrandMark(size: 52)));
}
