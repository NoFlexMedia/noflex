// SPDX-License-Identifier: AGPL-3.0-only
//
// Navigation is two navigators and no routes table. Pages push onto the content
// navigator inside the shell, so navigation stays visible on every window size;
// the player, sheets and dialogs go on the root navigator, above everything.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../layout/window.dart';
import '../theme.dart';

final rootNavigatorKey = GlobalKey<NavigatorState>();
final contentNavigatorKey = GlobalKey<NavigatorState>();

enum ShellTab { home, browse, search, downloads, account }

/// Destinations the navigation lists beyond its tabs. Everything opened from one
/// is still in it, so the navigation goes on showing it.
abstract final class NavPlace {
  static const collections = 'collections';
  static const liveTv = 'live-tv';
  static const watchRooms = 'watch-rooms';
  static const administration = 'administration';
  static String category(String name) => 'category:$name';
  static String library(String id, {String? server}) =>
      'library:${server ?? ''}:$id';
}

/// Marks a route as a place. Not a route name: nothing navigates by these.
final class _PlaceTag {
  const _PlaceTag(this.place);
  final String place;
}

String? _placeOf(Route<dynamic> route) => switch (route.settings.arguments) {
  _PlaceTag(:final place) => place,
  _ => null,
};

const _sidebarCollapsedKey = 'nav.sidebarCollapsed';

class NavService {
  final tab = signal(ShellTab.home);

  /// The innermost place on the content navigator, however it got there or left:
  /// a back gesture passes through here like anything else.
  final place = signal<String?>(null);

  late final placeObserver = _PlaceObserver(this);

  /// The tab a bar or rail should light, having no entry for a place itself.
  /// Each place lives under the tab that offers it there.
  late final shownTab = computed(
    () => switch (place.value) {
      null => tab.value,
      NavPlace.administration => ShellTab.account,
      _ => ShellTab.browse,
    },
  );

  /// Per device, like the web's per browser: it's about this screen.
  final sidebarCollapsed = signal(false);

  Future<void> load() async {
    final prefs = await SharedPreferences.getInstance();
    sidebarCollapsed.value = prefs.getBool(_sidebarCollapsedKey) ?? false;
  }

  void toggleSidebar() {
    final next = !sidebarCollapsed.value;
    sidebarCollapsed.value = next;
    unawaited(
      SharedPreferences.getInstance().then(
        (prefs) => prefs.setBool(_sidebarCollapsedKey, next),
      ),
    );
  }

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

  NavigatorState get _content =>
      contentNavigatorKey.currentState ?? rootNavigatorKey.currentState!;

  Future<T?> push<T>(Widget page, {String? place}) => _content.push(
    MaterialPageRoute<T>(
      settings: RouteSettings(
        arguments: place == null ? null : _PlaceTag(place),
      ),
      builder: (_) => page,
    ),
  );

  /// Go to a place from the navigation. It replaces whichever one you're in
  /// rather than stacking on it, and it fades, since it's a move sideways and not
  /// a step deeper. Going where you already are returns to its top, or starts it
  /// over if [fresh].
  void open(String place, Widget page, {bool fresh = false}) {
    if (!fresh && this.place.value == place) {
      _content.popUntil((route) => route.isFirst || _placeOf(route) == place);
      return;
    }
    unawaited(
      _content.pushAndRemoveUntil<void>(
        PageRouteBuilder(
          settings: RouteSettings(arguments: _PlaceTag(place)),
          transitionDuration: NfMotion.fast,
          reverseTransitionDuration: NfMotion.fast,
          pageBuilder: (_, _, _) => page,
          transitionsBuilder: (_, animation, _, child) =>
              FadeTransition(opacity: animation, child: child),
        ),
        (route) => route.isFirst,
      ),
    );
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

class _PlaceObserver extends NavigatorObserver {
  _PlaceObserver(this._nav);

  final NavService _nav;
  final _routes = <Route<dynamic>>[];

  void _sync() {
    String? place;
    for (final route in _routes.reversed) {
      place = _placeOf(route);
      if (place != null) break;
    }
    _nav.place.value = place;
  }

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    // The first route of a new shell's navigator, whose predecessor never said
    // goodbye. It's mid-build beside navigation that reads the place, so that
    // has to wait for the frame.
    if (previousRoute == null) {
      _routes
        ..clear()
        ..add(route);
      SchedulerBinding.instance.addPostFrameCallback((_) => _sync());
      return;
    }
    _routes.add(route);
    _sync();
  }

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) {
    _routes.remove(route);
    _sync();
  }

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) {
    _routes.remove(route);
    _sync();
  }

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) {
    final index = oldRoute == null ? -1 : _routes.indexOf(oldRoute);
    if (index >= 0 && newRoute != null) _routes[index] = newRoute;
    _sync();
  }
}
