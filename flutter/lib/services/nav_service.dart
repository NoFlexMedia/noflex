// SPDX-License-Identifier: AGPL-3.0-only
//
// Navigation is two navigators and no routes table. Pages push onto the content
// navigator inside the shell, so navigation stays visible on every window size;
// the player, sheets and dialogs go on the root navigator, above everything.

import 'package:flutter/material.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../layout/window.dart';
import '../theme.dart';

final rootNavigatorKey = GlobalKey<NavigatorState>();
final contentNavigatorKey = GlobalKey<NavigatorState>();

enum ShellTab { home, browse, search, downloads, account }

class NavService {
  final tab = signal(ShellTab.home);

  /// Bumped when a tab is re-selected, for its root to scroll home.
  final reselected = signal(0);

  void selectTab(ShellTab next) {
    if (tab.value == next) {
      contentNavigatorKey.currentState?.popUntil((route) => route.isFirst);
      reselected.value++;
      return;
    }
    contentNavigatorKey.currentState?.popUntil((route) => route.isFirst);
    tab.value = next;
  }

  Future<T?> push<T>(Widget page) {
    final navigator =
        contentNavigatorKey.currentState ?? rootNavigatorKey.currentState!;
    return navigator.push(MaterialPageRoute<T>(builder: (_) => page));
  }

  /// Above the shell, for the player and other full-window surfaces.
  Future<T?> pushFullWindow<T>(Widget page) =>
      rootNavigatorKey.currentState!.push(
        PageRouteBuilder<T>(
          opaque: true,
          transitionDuration: NfMotion.base,
          reverseTransitionDuration: NfMotion.fast,
          pageBuilder: (_, _, _) => page,
          transitionsBuilder: (_, animation, _, child) =>
              FadeTransition(opacity: animation, child: child),
        ),
      );

  /// A bottom sheet on a phone, a centered dialog once there's room.
  Future<T?> openSheet<T>(WidgetBuilder builder, {bool wide = false}) {
    final context = rootNavigatorKey.currentContext!;
    if (context.windowClass.isCompact) {
      return showModalBottomSheet<T>(
        context: context,
        useRootNavigator: true,
        isScrollControlled: true,
        useSafeArea: true,
        backgroundColor: NfColors.bgRaised,
        barrierColor: const Color(0xB8030406),
        shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(NfRadii.xl)),
        ),
        builder: builder,
      );
    }
    return showDialog<T>(
      context: context,
      useRootNavigator: true,
      barrierColor: const Color(0xB8030406),
      builder: (context) => Dialog(
        backgroundColor: NfColors.bgRaised,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(NfRadii.l),
          side: const BorderSide(color: NfColors.lineStrong),
        ),
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxWidth: wide ? 640 : 460,
            maxHeight: 640,
          ),
          child: builder(context),
        ),
      ),
    );
  }

  void close<T>([T? result]) => rootNavigatorKey.currentState?.pop(result);
}
