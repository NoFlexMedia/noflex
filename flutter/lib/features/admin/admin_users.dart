// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/admin.data.gql.dart';
import '../../graphql/queries/__generated__/create_invite.req.gql.dart';
import '../../graphql/queries/__generated__/create_user.req.gql.dart';
import '../../graphql/queries/__generated__/delete_user.req.gql.dart';
import '../../graphql/queries/__generated__/parental_tags.data.gql.dart';
import '../../graphql/queries/__generated__/parental_tags.req.gql.dart';
import '../../graphql/queries/__generated__/revoke_session.req.gql.dart';
import '../../graphql/queries/__generated__/revoke_user_sessions.req.gql.dart';
import '../../graphql/queries/__generated__/set_user_restrictions.req.gql.dart';
import '../../graphql/queries/__generated__/set_viewer_device_smart_transcode.req.gql.dart';
import '../../graphql/queries/__generated__/update_user.req.gql.dart';
import '../../graphql/queries/__generated__/user_devices.data.gql.dart';
import '../../graphql/queries/__generated__/user_devices.req.gql.dart';
import '../../graphql/queries/__generated__/user_restrictions.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/admin_library_fields.data.gql.dart';
import '../../graphql/queries/fragments/__generated__/admin_user_fields.data.gql.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
import '../../theme.dart';
import '../../utils/device_icon.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../../widgets/user_avatar.dart';
import '../account/account_widgets.dart';
import 'admin_controller.dart';
import 'admin_format.dart';
import 'admin_widgets.dart';

typedef _User = GAdminUserFields;
typedef _Library = GAdminLibraryFields;
typedef _Session = GAdminData_sessions;

const _roles = [
  NfOption(GUserRole.USER, 'User', detail: 'Watches granted libraries'),
  NfOption(GUserRole.GUEST, 'Guest', detail: 'Limited, read-only'),
  NfOption(GUserRole.ADMIN, 'Administrator', detail: 'Manages the server'),
];

const _sessionPreview = 5;
const _sessionPageSize = 20;

/// Most recently seen first.
List<_Session> _sessionsOf(GAdminData? data) =>
    (data?.sessions.toList() ?? <_Session>[])
      ..sort((a, b) => b.lastSeenAt.compareTo(a.lastSeenAt));

bool _isActive(_Session session) =>
    DateTime.now().difference(session.lastSeenAt) < sessionActiveWindow;

String _device(_Session session) =>
    (session.device?.isEmpty ?? true) ? 'Unknown device' : session.device!;

class UsersSection extends StatelessWidget {
  const UsersSection({super.key, required this.controller});
  final AdminController controller;

  Future<void> _create(List<_Library> libraries) async {
    final user = await getIt<NavService>().openSheet<_User>(
      (context) => const _NewUserSheet(),
    );
    if (user == null) return;
    controller.admin.refetch();
    if (user.role != GUserRole.ADMIN && libraries.isNotEmpty) {
      await _access(user, libraries);
    }
  }

  Future<void> _access(_User user, List<_Library> libraries) async {
    final saved = await getIt<NavService>().openSheet<bool>(
      (context) => _AccessSheet(user: user, libraries: libraries),
    );
    if (saved == true) controller.admin.refetch();
  }

  Future<void> _revokeAll(String id, String name) async {
    final confirmed = await confirmAction(
      title: 'Sign $name out everywhere?',
      message: 'Every device they’re using will need to sign in again.',
      confirmLabel: 'Sign out everywhere',
      danger: true,
    );
    if (!confirmed) return;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GRevokeUserSessionsReq((b) => b.vars.user = id),
      ),
      success: '$name signed out everywhere',
    );
    if (done != null) controller.admin.refetch();
  }

  Future<void> _remove(_User user) async {
    final confirmed = await confirmAction(
      title: 'Delete ${user.name}?',
      message: 'Their account, sessions, playlists and watch history are removed. This can’t be undone.',
      confirmLabel: 'Delete user',
      danger: true,
    );
    if (!confirmed) return;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GDeleteUserReq((b) => b.vars.id = user.id),
      ),
      success: '${user.name} deleted',
    );
    if (done != null) controller.admin.refetch();
  }

  Future<void> _revokeSession(_Session session, String who) async {
    final confirmed = await confirmAction(
      title:
          'Sign out ${(session.device?.isEmpty ?? true) ? 'this device' : session.device}?',
      message: '$who will need to sign in again on it.',
      confirmLabel: 'Sign out device',
      danger: true,
    );
    if (!confirmed) return;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GRevokeSessionReq((b) => b.vars.id = session.id),
      ),
      success: 'Session revoked',
    );
    if (done != null) controller.admin.refetch();
  }

  Future<void> _menu(
    _User user,
    List<_Library> libraries, {
    required bool signedIn,
    required bool isYou,
  }) async {
    final action = await pickOption<String>(
      title: user.name,
      options: [
        const NfOption(
          'access',
          'Role & library access…',
          leading: Icon(PhosphorIconsRegular.books, size: 18),
        ),
        const NfOption(
          'restrict',
          'Restrictions…',
          leading: Icon(PhosphorIconsRegular.shieldCheck, size: 18),
        ),
        const NfOption(
          'devices',
          'Devices…',
          leading: Icon(PhosphorIconsRegular.devices, size: 18),
        ),
        if (signedIn)
          const NfOption(
            'signout',
            'Sign out everywhere',
            leading: Icon(PhosphorIconsRegular.signOut, size: 18),
          ),
        if (!isYou)
          const NfOption(
            'delete',
            'Delete user',
            leading: Icon(
              PhosphorIconsRegular.trash,
              size: 18,
              color: NfColors.danger,
            ),
          ),
      ],
    );
    switch (action) {
      case 'access':
        await _access(user, libraries);
      case 'restrict':
        await getIt<NavService>().openSheet<void>(
          (context) => _RestrictionsSheet(user: user),
        );
      case 'devices':
        await getIt<NavService>().openSheet<void>(
          (context) => _DevicesSheet(user: user),
        );
      case 'signout':
        await _revokeAll(user.id, user.name);
      case 'delete':
        await _remove(user);
    }
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      if (controller.admin.waiting.value) {
        return const NfSkeletonRows();
      }
      final data = controller.admin.data.value;
      if (data == null) {
        return NfState.error(
          title: 'Couldn’t read the server’s state',
          message: controller.admin.error.value?.toString(),
          retry: controller.admin.refetch,
        );
      }
      final viewerId = getIt<SessionService>().viewer.value?.id;
      final libraries = data.libraries.toList().cast<_Library>();
      final users = data.users.toList()
        ..sort((a, b) {
          if ((a.id == viewerId) != (b.id == viewerId)) {
            return a.id == viewerId ? -1 : 1;
          }
          return a.name.compareTo(b.name);
        });
      final sessions = _sessionsOf(data);
      final names = {for (final user in users) user.id: user.name};
      final signedIn = <String, int>{};
      for (final session in sessions) {
        signedIn.update(session.userId, (n) => n + 1, ifAbsent: () => 1);
      }
      String grants(_User user) => user.role == GUserRole.ADMIN
          ? 'All libraries'
          : '${user.libraryAccess.length} of ${plural(libraries.length, 'library', 'libraries')}';
      return AdminColumns(
        minWidth: 460,
        children: [
          AccountPanel(
            icon: PhosphorIconsRegular.users,
            title: 'Accounts',
            detail:
                '${users.length} ${users.length == 1 ? 'person' : 'people'} can sign in',
            child: AdminStack(
              gap: 4,
              children: [
                Wrap(
                  spacing: 8,
                  children: [
                    NfButton(
                      label: 'Invite',
                      size: NfButtonSize.small,
                      icon: const Icon(PhosphorIconsRegular.users),
                      onPressed: () => unawaited(
                        getIt<NavService>().openSheet<void>(
                          (context) => _InviteSheet(libraries: libraries),
                        ),
                      ),
                    ),
                    NfButton(
                      label: 'New user',
                      size: NfButtonSize.small,
                      icon: const Icon(PhosphorIconsRegular.plus),
                      onPressed: () => unawaited(_create(libraries)),
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                for (final user in users)
                  Padding(
                    padding: const EdgeInsets.symmetric(vertical: 6),
                    child: Row(
                      children: [
                        UserAvatar(
                          name: user.name,
                          avatarUrl: user.avatarUrl,
                          size: 36,
                        ),
                        const SizedBox(width: 12),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Wrap(
                                spacing: 8,
                                crossAxisAlignment: WrapCrossAlignment.center,
                                children: [
                                  Text(
                                    user.name,
                                    style: nfText(14.5, weight: 600),
                                  ),
                                  if (user.id == viewerId) const TagChip('You'),
                                ],
                              ),
                              Text(
                                [
                                  if (user.username != user.name) user.username,
                                  grants(user),
                                  'joined ${formatDate(user.createdAt)}',
                                  if (signedIn[user.id] != null)
                                    '${signedIn[user.id]} signed in',
                                ].join(' · '),
                                style: NfType.caption,
                              ),
                            ],
                          ),
                        ),
                        const SizedBox(width: 8),
                        StatePill(
                          titleCase(user.role.name),
                          tone: user.role == GUserRole.ADMIN
                              ? StateTone.accent
                              : StateTone.none,
                        ),
                        NfIconButton(
                          icon: PhosphorIconsBold.dotsThree,
                          label: 'Manage ${user.name}',
                          size: 34,
                          iconSize: 18,
                          onPressed: () => unawaited(
                            _menu(
                              user,
                              libraries,
                              signedIn: signedIn[user.id] != null,
                              isYou: user.id == viewerId,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
              ],
            ),
          ),
          _SessionsPanel(
            controller: controller,
            sessions: sessions,
            names: names,
            onRefresh: controller.admin.refetch,
            onRevoke: _revokeSession,
            onRevokeAll: _revokeAll,
          ),
        ],
      );
    },
  );
}

class _SessionRow extends StatelessWidget {
  const _SessionRow({
    required this.session,
    required this.owner,
    required this.withOwner,
    required this.onRevoke,
  });

  final _Session session;
  final String owner;
  final bool withOwner;
  final VoidCallback onRevoke;

  @override
  Widget build(BuildContext context) {
    final active = _isActive(session);
    final when = active ? 'active now' : relative(session.lastSeenAt);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 6),
      child: Row(
        children: [
          Stack(
            clipBehavior: Clip.none,
            children: [
              AccountGlyph(
                icon: PhosphorIconsRegular.devices,
                color: active ? NfColors.text : NfColors.text2,
              ),
              if (active)
                Positioned(
                  right: -2,
                  bottom: -2,
                  child: Container(
                    width: 11,
                    height: 11,
                    decoration: BoxDecoration(
                      color: NfColors.success,
                      shape: BoxShape.circle,
                      border: Border.all(color: NfColors.bgRaised, width: 2),
                    ),
                  ),
                ),
            ],
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  withOwner ? owner : _device(session),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: nfText(14, weight: 600),
                ),
                Tooltip(
                  message: stamp(session.lastSeenAt),
                  child: Text(
                    [
                      if (withOwner) _device(session),
                      if (!withOwner && (session.client?.isNotEmpty ?? false))
                        session.client!,
                      when,
                    ].join(' · '),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: NfType.caption,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: 8),
          Text(
            (session.remoteAddr?.isEmpty ?? true)
                ? 'local'
                : session.remoteAddr!,
            style: NfType.mono.copyWith(fontSize: 11.5, color: NfColors.text3),
          ),
          NfIconButton(
            icon: PhosphorIconsRegular.x,
            label: 'Sign out $owner’s session on ${_device(session)}',
            size: 34,
            iconSize: 16,
            onPressed: onRevoke,
          ),
        ],
      ),
    );
  }
}

class _SessionsPanel extends StatelessWidget {
  const _SessionsPanel({
    required this.controller,
    required this.sessions,
    required this.names,
    required this.onRefresh,
    required this.onRevoke,
    required this.onRevokeAll,
  });

  final AdminController controller;
  final List<_Session> sessions;
  final Map<String, String> names;
  final VoidCallback onRefresh;
  final Future<void> Function(_Session, String) onRevoke;
  final Future<void> Function(String, String) onRevokeAll;

  String _owner(_Session session) => names[session.userId] ?? 'Deleted user';

  @override
  Widget build(BuildContext context) {
    final active = sessions.where(_isActive).length;
    final people = sessions.map((s) => s.userId).toSet().length;
    return AccountPanel(
      icon: PhosphorIconsRegular.devices,
      title: 'Active sessions',
      detail: sessions.isEmpty
          ? 'Every signed-in device on this server'
          : '${sessions.length} signed in · $active active now · $people ${people == 1 ? 'person' : 'people'}',
      trailing: NfIconButton(
        icon: PhosphorIconsRegular.arrowsClockwise,
        label: 'Refresh sessions',
        size: 34,
        iconSize: 16,
        onPressed: onRefresh,
      ),
      child: sessions.isEmpty
          ? Text('Nobody is signed in right now.', style: NfType.caption)
          : Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (final session in sessions.take(_sessionPreview))
                  _SessionRow(
                    session: session,
                    owner: _owner(session),
                    withOwner: true,
                    onRevoke: () =>
                        unawaited(onRevoke(session, _owner(session))),
                  ),
                if (sessions.length > _sessionPreview)
                  Padding(
                    padding: const EdgeInsets.only(top: 8),
                    child: NfButton(
                      label: 'View all ${sessions.length} sessions',
                      style: NfButtonStyle.ghost,
                      size: NfButtonSize.small,
                      icon: const Icon(PhosphorIconsRegular.caretRight),
                      onPressed: () => unawaited(
                        getIt<NavService>().openSheet<void>(
                          (context) => _AllSessionsSheet(
                            controller: controller,
                            onRevoke: onRevoke,
                            onRevokeAll: onRevokeAll,
                          ),
                          wide: true,
                        ),
                      ),
                    ),
                  ),
              ],
            ),
    );
  }
}

enum _SessionFilter { all, active, idle }

class _AllSessionsSheet extends StatefulWidget {
  const _AllSessionsSheet({
    required this.controller,
    required this.onRevoke,
    required this.onRevokeAll,
  });

  final AdminController controller;
  final Future<void> Function(_Session, String) onRevoke;
  final Future<void> Function(String, String) onRevokeAll;

  @override
  State<_AllSessionsSheet> createState() => _AllSessionsSheetState();
}

class _AllSessionsSheetState extends State<_AllSessionsSheet> {
  final _search = TextEditingController();
  final _text = signal('');
  final _show = signal(_SessionFilter.all);
  final _page = signal(0);

  // Read from the admin query, so a device signed out here leaves the list.
  late final _sessions = computed(
    () => _sessionsOf(widget.controller.admin.data.value),
  );
  late final _names = computed(() {
    final data = widget.controller.admin.data.value;
    return {
      if (data != null)
        for (final user in data.users) user.id: user.name,
    };
  });

  @override
  void dispose() {
    _search.dispose();
    super.dispose();
  }

  String _owner(_Session session) =>
      _names.value[session.userId] ?? 'Deleted user';

  @override
  Widget build(BuildContext context) {
    return AdminSheet(
      title: 'Active sessions',
      footer: [
        SignalBuilder(
          builder: (context) {
            final count = _matching().length;
            final pages = _pageCount(count);
            final page = _page.value < pages ? _page.value : pages - 1;
            if (pages <= 1) return const SizedBox.shrink();
            final last = (page + 1) * _sessionPageSize;
            return Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                NfIconButton(
                  icon: PhosphorIconsRegular.caretLeft,
                  label: 'Previous page',
                  size: 34,
                  iconSize: 16,
                  onPressed: page == 0 ? null : () => _page.value = page - 1,
                ),
                Text(
                  '${page * _sessionPageSize + 1}–${last < count ? last : count} of $count',
                  style: NfType.small,
                ),
                NfIconButton(
                  icon: PhosphorIconsRegular.caretRight,
                  label: 'Next page',
                  size: 34,
                  iconSize: 16,
                  onPressed: page >= pages - 1
                      ? null
                      : () => _page.value = page + 1,
                ),
              ],
            );
          },
        ),
        NfButton(
          label: 'Close',
          style: NfButtonStyle.ghost,
          onPressed: () => Navigator.of(context).pop(),
        ),
      ],
      child: SignalBuilder(
        builder: (context) {
          final matching = _matching();
          final pages = _pageCount(matching.length);
          // Signing devices out can shrink the list from under the last page.
          final page = _page.value < pages ? _page.value : pages - 1;
          final groups = groupSessions(
            matching,
            userOf: (session) => session.userId,
            page: page,
            pageSize: _sessionPageSize,
          );
          final sessions = _sessions.value;
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                '${sessions.length} signed in · ${sessions.where(_isActive).length} active now',
                style: NfType.small,
              ),
              const SizedBox(height: 12),
              NfField(
                controller: _search,
                hint: 'Person, device or address',
                autocorrect: false,
                prefixIcon: const Icon(
                  PhosphorIconsRegular.magnifyingGlass,
                  size: 16,
                  color: NfColors.text3,
                ),
                onChanged: (value) => batch(() {
                  _text.value = value;
                  _page.value = 0;
                }),
              ),
              const SizedBox(height: 10),
              AdminSegments<_SessionFilter>(
                items: const [
                  (_SessionFilter.all, 'All'),
                  (_SessionFilter.active, 'Active now'),
                  (_SessionFilter.idle, 'Idle'),
                ],
                value: _show.value,
                onChanged: (value) => batch(() {
                  _show.value = value;
                  _page.value = 0;
                }),
              ),
              const SizedBox(height: 14),
              if (groups.isEmpty)
                Text('No sessions match.', style: NfType.caption),
              for (final group in groups)
                Padding(
                  padding: const EdgeInsets.only(bottom: 14),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Row(
                        children: [
                          Expanded(
                            child: Text.rich(
                              TextSpan(
                                children: [
                                  TextSpan(
                                    text:
                                        '${_names.value[group.userId] ?? 'Deleted user'}  ',
                                    style: nfText(14.5, weight: 650),
                                  ),
                                  TextSpan(
                                    text: plural(group.total, 'session'),
                                    style: NfType.caption,
                                  ),
                                ],
                              ),
                            ),
                          ),
                          if (_names.value.containsKey(group.userId))
                            NfButton(
                              label: 'Sign out everywhere',
                              style: NfButtonStyle.ghost,
                              size: NfButtonSize.small,
                              icon: const Icon(PhosphorIconsRegular.signOut),
                              onPressed: () =>
                                  unawaited(_revokeAll(group.userId)),
                            ),
                        ],
                      ),
                      for (final session in group.sessions)
                        _SessionRow(
                          session: session,
                          owner: _owner(session),
                          withOwner: false,
                          onRevoke: () => unawaited(_revoke(session)),
                        ),
                    ],
                  ),
                ),
            ],
          );
        },
      ),
    );
  }

  int _pageCount(int count) {
    final pages = (count / _sessionPageSize).ceil();
    return pages < 1 ? 1 : pages;
  }

  List<_Session> _matching() {
    final words = _text.value
        .trim()
        .toLowerCase()
        .split(RegExp(r'\s+'))
        .where((word) => word.isNotEmpty)
        .toList();
    final show = _show.value;
    return [
      for (final session in _sessions.value)
        if ((show == _SessionFilter.all ||
                _isActive(session) == (show == _SessionFilter.active)) &&
            words.every(
              '${_owner(session)} ${_device(session)} ${session.client ?? ''} ${session.remoteAddr ?? ''}'
                  .toLowerCase()
                  .contains,
            ))
          session,
    ];
  }

  Future<void> _revoke(_Session session) =>
      widget.onRevoke(session, _owner(session));

  Future<void> _revokeAll(String userId) =>
      widget.onRevokeAll(userId, _names.value[userId] ?? 'Deleted user');
}

class _NewUserSheet extends StatefulWidget {
  const _NewUserSheet();

  @override
  State<_NewUserSheet> createState() => _NewUserSheetState();
}

class _NewUserSheetState extends State<_NewUserSheet> {
  final _username = TextEditingController();
  final _password = TextEditingController();
  final _role = signal(GUserRole.USER);
  final _saving = signal(false);
  final _ready = signal(false);

  @override
  void dispose() {
    _username.dispose();
    _password.dispose();
    super.dispose();
  }

  void _check() => _ready.value =
      _username.text.trim().isNotEmpty && _password.text.length >= 8;

  Future<void> _create() async {
    final username = _username.text.trim();
    _saving.value = true;
    final data = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GCreateUserReq(
          (b) => b.vars
            ..username = username
            ..password = _password.text
            ..role = _role.value,
        ),
      ),
      success: '$username can now sign in',
    );
    if (!mounted) return;
    _saving.value = false;
    if (data != null) Navigator.of(context).pop<_User>(data.createUser);
  }

  @override
  Widget build(BuildContext context) => AdminSheet(
    title: 'New user',
    description: 'They can change the password after signing in.',
    footer: [
      NfButton(
        label: 'Cancel',
        style: NfButtonStyle.ghost,
        onPressed: () => Navigator.of(context).pop(),
      ),
      SignalBuilder(
        builder: (context) => NfButton(
          label: 'Create user',
          style: NfButtonStyle.primary,
          busy: _saving.value,
          onPressed: _ready.value ? () => unawaited(_create()) : null,
        ),
      ),
    ],
    child: AdminStack(
      gap: 12,
      children: [
        NfField(
          controller: _username,
          label: 'Username',
          autocorrect: false,
          autofocus: true,
          onChanged: (_) => _check(),
        ),
        Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            NfField(
              controller: _password,
              label: 'Initial password',
              obscure: true,
              autocorrect: false,
              onChanged: (_) => _check(),
            ),
            const FieldNote('At least 8 characters.'),
          ],
        ),
        SignalBuilder(
          builder: (context) => AdminSelect<GUserRole>(
            label: 'Role',
            options: _roles,
            value: _role.value,
            onChanged: (value) => _role.value = value,
          ),
        ),
      ],
    ),
  );
}

class _AccessSheet extends StatefulWidget {
  const _AccessSheet({required this.user, required this.libraries});
  final _User user;
  final List<_Library> libraries;

  @override
  State<_AccessSheet> createState() => _AccessSheetState();
}

class _AccessSheetState extends State<_AccessSheet> {
  late final Signal<GUserRole> _role;
  late final Signal<Set<String>> _granted;
  final _saving = signal(false);

  @override
  void initState() {
    super.initState();
    _role = signal(widget.user.role);
    _granted = signal(widget.user.libraryAccess.toSet());
  }

  Future<void> _save() async {
    _saving.value = true;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GUpdateUserReq(
          (b) => b.vars
            ..id = widget.user.id
            ..role = _role.value
            ..libraries.replace(_granted.value),
        ),
      ),
      success: 'Access updated for ${widget.user.name}',
    );
    if (!mounted) return;
    _saving.value = false;
    if (done != null) Navigator.of(context).pop(true);
  }

  @override
  Widget build(BuildContext context) => AdminSheet(
    title: 'Access for ${widget.user.name}',
    description: 'Choose a role and which libraries they can browse.',
    footer: [
      NfButton(
        label: 'Cancel',
        style: NfButtonStyle.ghost,
        onPressed: () => Navigator.of(context).pop(false),
      ),
      SignalBuilder(
        builder: (context) => NfButton(
          label: 'Save access',
          style: NfButtonStyle.primary,
          busy: _saving.value,
          onPressed: () => unawaited(_save()),
        ),
      ),
    ],
    child: SignalBuilder(
      builder: (context) {
        final granted = _granted.value;
        return AdminStack(
          gap: 10,
          children: [
            AdminSelect<GUserRole>(
              label: 'Role',
              options: _roles,
              value: _role.value,
              onChanged: (value) => _role.value = value,
            ),
            const SizedBox(height: 4),
            const FormHeading('Libraries'),
            if (_role.value == GUserRole.ADMIN)
              Text(
                'Administrators can see every library; these grants apply if the role changes later.',
                style: NfType.small,
              ),
            if (widget.libraries.isEmpty)
              Text('No libraries exist yet.', style: NfType.small),
            for (final library in widget.libraries)
              AdminCheck(
                label: library.name,
                detail: titleCase(library.kind.name),
                value: granted.contains(library.id),
                onChanged: (value) => _granted.value = value
                    ? {...granted, library.id}
                    : ({...granted}..remove(library.id)),
              ),
            if (widget.libraries.length > 3)
              Wrap(
                spacing: 8,
                children: [
                  NfButton(
                    label: 'Select all',
                    style: NfButtonStyle.ghost,
                    size: NfButtonSize.small,
                    onPressed: () => _granted.value = {
                      for (final library in widget.libraries) library.id,
                    },
                  ),
                  NfButton(
                    label: 'None',
                    style: NfButtonStyle.ghost,
                    size: NfButtonSize.small,
                    onPressed: () => _granted.value = const {},
                  ),
                ],
              ),
          ],
        );
      },
    ),
  );
}

class _InviteSheet extends StatefulWidget {
  const _InviteSheet({required this.libraries});
  final List<_Library> libraries;

  @override
  State<_InviteSheet> createState() => _InviteSheetState();
}

class _InviteSheetState extends State<_InviteSheet> {
  final _days = TextEditingController(text: '7');
  final _uses = TextEditingController(text: '1');
  final _chosen = signal<Set<String>>(const {});
  final _link = signal<String?>(null);
  final _saving = signal(false);

  @override
  void dispose() {
    _days.dispose();
    _uses.dispose();
    super.dispose();
  }

  Future<void> _create() async {
    final days = int.tryParse(_days.text.trim());
    final uses = int.tryParse(_uses.text.trim());
    if (days == null || uses == null) return;
    _saving.value = true;
    final data = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GCreateInviteReq(
          (b) => b.vars
            ..libraries.replace(_chosen.value)
            ..days = days
            ..uses = uses,
        ),
      ),
    );
    if (!mounted) return;
    _saving.value = false;
    if (data != null) {
      final origin = getIt<TokenStore>().origin.value;
      _link.value = '$origin/invite#${data.createInvite.token}';
    }
  }

  Future<void> _copy() async {
    final link = _link.value;
    if (link == null) return;
    await Clipboard.setData(ClipboardData(text: link));
    getIt<ToastService>().show('Invite link copied', kind: ToastKind.success);
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final link = _link.value;
      final chosen = _chosen.value;
      return AdminSheet(
        title: 'Invite someone',
        description: 'They choose their own username and password, and the account is never an administrator.',
        footer: link != null
            ? [
                NfButton(
                  label: 'Done',
                  style: NfButtonStyle.ghost,
                  onPressed: () => Navigator.of(context).pop(),
                ),
                NfButton(
                  label: 'Copy link',
                  style: NfButtonStyle.primary,
                  onPressed: () => unawaited(_copy()),
                ),
              ]
            : [
                NfButton(
                  label: 'Cancel',
                  style: NfButtonStyle.ghost,
                  onPressed: () => Navigator.of(context).pop(),
                ),
                NfButton(
                  label: 'Create link',
                  style: NfButtonStyle.primary,
                  busy: _saving.value,
                  onPressed: () => unawaited(_create()),
                ),
              ],
        child: link != null
            ? Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'Invite link',
                    style: nfText(13, weight: 600, color: NfColors.text2),
                  ),
                  const SizedBox(height: 6),
                  SelectableText(link, style: NfType.mono),
                  const FieldNote(
                    'Shown once. Anyone with this link can make an account until it expires or is used up.',
                  ),
                ],
              )
            : AdminStack(
                gap: 10,
                children: [
                  const FormHeading('Libraries they can see'),
                  if (widget.libraries.isEmpty)
                    Text('No libraries exist yet.', style: NfType.small),
                  for (final library in widget.libraries)
                    AdminCheck(
                      label: library.name,
                      value: chosen.contains(library.id),
                      onChanged: (value) => _chosen.value = value
                          ? {...chosen, library.id}
                          : ({...chosen}..remove(library.id)),
                    ),
                  NfField(
                    controller: _days,
                    label: 'Expires after (days)',
                    keyboardType: TextInputType.number,
                  ),
                  NfField(
                    controller: _uses,
                    label: 'Number of people',
                    keyboardType: TextInputType.number,
                  ),
                ],
              ),
      );
    },
  );
}

typedef _RatingTag = GParentalTagsData_tags_nodes_tag;

class _RestrictionsSheet extends StatefulWidget {
  const _RestrictionsSheet({required this.user});
  final _User user;

  @override
  State<_RestrictionsSheet> createState() => _RestrictionsSheetState();
}

class _RestrictionsSheetState extends State<_RestrictionsSheet> {
  final _extra = TextEditingController();
  final _bitrate = TextEditingController();
  final _transcodes = TextEditingController();
  final _tags = signal<List<_RatingTag>>(const []);
  final _ids = signal<Set<String>>(const {});
  final _allowUnrated = signal(true);
  final _loaded = signal(false);
  final _error = signal<String?>(null);
  final _saving = signal(false);
  final _showManual = signal(false);

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  @override
  void dispose() {
    _extra.dispose();
    _bitrate.dispose();
    _transcodes.dispose();
    super.dispose();
  }

  Future<void> _load() async {
    final graphql = getIt<GraphqlService>();
    try {
      final restrictions = await graphql.run(
        GUserRestrictionsReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NoCache
            ..vars.user = widget.user.id,
        ),
      );
      final tags = await graphql.run(GParentalTagsReq());
      if (!mounted) return;
      final current = restrictions.userRestrictions;
      final bps = current.maxStreamBitrateBps;
      _bitrate.text = bps == null || bps == 0
          ? ''
          : '${(bps / 100000).round() / 10}';
      _transcodes.text = current.maxConcurrentTranscodes?.toString() ?? '';
      batch(() {
        _tags.value = [for (final node in tags.tags.nodes) node.tag];
        _ids.value = current.allowedParentalRatingIds.toSet();
        _allowUnrated.value = current.allowUnrated;
        _loaded.value = true;
      });
    } on Exception catch (cause) {
      if (mounted) _error.value = cause.toString();
    }
  }

  Future<void> _save() async {
    final bitrate = double.tryParse(_bitrate.text.trim());
    final ids = {..._ids.value, ...splitList(_extra.text)};
    _saving.value = true;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GSetUserRestrictionsReq(
          (b) => b.vars
            ..user = widget.user.id
            ..input.allowedParentalRatingIds.replace(ids)
            ..input.allowUnrated = _allowUnrated.value
            ..input.maxStreamBitrateBps = bitrate == null
                ? null
                : (bitrate * 1000000).round()
            ..input.maxConcurrentTranscodes = int.tryParse(
              _transcodes.text.trim(),
            ),
        ),
      ),
      success: 'Restrictions saved for ${widget.user.name}',
    );
    if (!mounted) return;
    _saving.value = false;
    if (done != null) Navigator.of(context).pop();
  }

  @override
  Widget build(BuildContext context) => AdminSheet(
    title: 'Restrictions for ${widget.user.name}',
    description: 'Limits apply on every device this person signs in from.',
    footer: [
      NfButton(
        label: 'Cancel',
        style: NfButtonStyle.ghost,
        onPressed: () => Navigator.of(context).pop(),
      ),
      SignalBuilder(
        builder: (context) => NfButton(
          label: 'Save restrictions',
          style: NfButtonStyle.primary,
          busy: _saving.value,
          onPressed: _loaded.value ? () => unawaited(_save()) : null,
        ),
      ),
    ],
    child: SignalBuilder(
      builder: (context) {
        final error = _error.value;
        if (error != null) return Notice(error);
        if (!_loaded.value) {
          return const NfSkeletonRows();
        }
        final ids = _ids.value;
        final tags = _tags.value;
        final known = {for (final tag in tags) tag.id};
        final orphans = ids.where((id) => !known.contains(id)).length;
        return AdminStack(
          gap: 12,
          children: [
            const FormHeading('Content ratings'),
            Text(
              ids.isEmpty
                  ? 'Nothing selected — every rating is allowed.'
                  : 'Only titles rated ${ids.length == 1 ? 'this way' : 'one of these ways'} are visible.',
              style: NfType.small,
            ),
            if (tags.isEmpty)
              Text(
                'No ratings have been tagged in your libraries yet.',
                style: NfType.small,
              )
            else
              Wrap(
                spacing: 6,
                runSpacing: 6,
                children: [
                  for (final tag in tags)
                    NfChoiceChip(
                      label: tag.display,
                      selected: ids.contains(tag.id),
                      onTap: () => _ids.value = ids.contains(tag.id)
                          ? ({...ids}..remove(tag.id))
                          : {...ids, tag.id},
                    ),
                ],
              ),
            if (orphans > 0)
              Row(
                children: [
                  Expanded(
                    child: Text(
                      'Also allowed: ${plural(orphans, 'rating ID')} not found in any library.',
                      style: NfType.small,
                    ),
                  ),
                  NfButton(
                    label: 'Remove',
                    style: NfButtonStyle.ghost,
                    size: NfButtonSize.small,
                    onPressed: () =>
                        _ids.value = ids.where(known.contains).toSet(),
                  ),
                ],
              ),
            Align(
              alignment: Alignment.centerLeft,
              child: NfButton(
                label: 'Add rating IDs manually',
                style: NfButtonStyle.ghost,
                size: NfButtonSize.small,
                onPressed: () => _showManual.value = !_showManual.value,
              ),
            ),
            if (_showManual.value)
              NfField(
                controller: _extra,
                label: 'Tag IDs',
                hint: 'One UUID per line',
                maxLines: 2,
                autocorrect: false,
              ),
            AccountToggle(
              title: 'Allow unrated titles',
              detail: 'Titles without any rating tag',
              value: _allowUnrated.value,
              onChanged: (value) => _allowUnrated.value = value,
            ),
            const SizedBox(height: 4),
            const FormHeading('Streaming'),
            NfField(
              controller: _bitrate,
              label: 'Max bitrate (Mbps)',
              hint: 'No cap',
              keyboardType: const TextInputType.numberWithOptions(
                decimal: true,
              ),
            ),
            NfField(
              controller: _transcodes,
              label: 'Simultaneous transcodes',
              hint: 'Server default',
              keyboardType: TextInputType.number,
            ),
          ],
        );
      },
    ),
  );
}

class _DevicesSheet extends StatefulWidget {
  const _DevicesSheet({required this.user});
  final _User user;

  @override
  State<_DevicesSheet> createState() => _DevicesSheetState();
}

class _DevicesSheetState extends State<_DevicesSheet> {
  final _devices = signal<List<GUserDevicesData_userDevices>?>(null);
  final _error = signal<String?>(null);

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    try {
      final data = await getIt<GraphqlService>().run(
        GUserDevicesReq((b) => b.vars.user = widget.user.id),
      );
      if (mounted) _devices.value = data.userDevices.toList();
    } catch (error) {
      if (mounted) _error.value = '$error';
    }
  }

  Future<void> _set(GUserDevicesData_userDevices device, bool enabled) async {
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GSetViewerDeviceSmartTranscodeReq(
          (b) => b.vars
            ..id = device.id
            ..enabled = enabled,
        ),
      ),
    );
    if (done != null) await _load();
  }

  @override
  Widget build(BuildContext context) => AdminSheet(
    title: '${widget.user.name}’s devices',
    description:
        'Turning smart transcoding off stops the server preparing what it '
        'predicts a device will watch. They can turn it back on, and anything '
        'they’ve marked for the device stays marked.',
    footer: [
      NfButton(
        label: 'Done',
        style: NfButtonStyle.ghost,
        onPressed: () => Navigator.of(context).pop(),
      ),
    ],
    child: SignalBuilder(
      builder: (context) {
        final error = _error.value;
        if (error != null) return Notice(error);
        final devices = _devices.value;
        if (devices == null) return const NfSkeletonRows();
        if (devices.isEmpty) {
          return Text(
            'No devices have registered with this account yet.',
            style: NfType.small,
          );
        }
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final device in devices)
              Row(
                children: [
                  Icon(
                    deviceIconFor(device.name),
                    size: 20,
                    color: NfColors.text2,
                  ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: AccountToggle(
                      title: device.name,
                      detail: 'Smart transcoding',
                      value: device.smartTranscode || device.smartDownloads,
                      onChanged: (enabled) => unawaited(_set(device, enabled)),
                    ),
                  ),
                ],
              ),
          ],
        );
      },
    ),
  );
}
