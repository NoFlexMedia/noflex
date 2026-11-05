// SPDX-License-Identifier: AGPL-3.0-only
//
// Rooms are only listed to their members, so someone with an invite joins by
// pasting it here. An open room is a waiting area: the host sees who's arrived
// and starts when ready, and guests wait, then go in by themselves when the host
// presses play.

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/control_watch_room.req.gql.dart';
import '../../graphql/queries/__generated__/join_watch_room.req.gql.dart';
import '../../graphql/queries/__generated__/leave_watch_room.req.gql.dart';
import '../../graphql/queries/__generated__/watch_rooms.data.gql.dart';
import '../../graphql/queries/__generated__/watch_rooms.req.gql.dart';
import '../../graphql/queries/__generated__/watch_rooms.var.gql.dart';
import '../../graphql/queries/fragments/__generated__/watch_room_fields.data.gql.dart';
import '../../graphql/query.dart';
import '../../layout/window.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../utils/images.dart';
import '../../widgets/artwork.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../browse/work_browser.dart';
import '../player/playback_service.dart';

/// There's no subscription for the room list or who's in a room. A waiting area
/// should notice arrivals, and the host pressing play, about when they happen.
const _roomPoll = Duration(seconds: 2);

final _roomId = RegExp(
  r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}',
  caseSensitive: false,
);

class WatchRoomsScreen extends StatefulWidget {
  const WatchRoomsScreen({super.key, this.roomId});

  /// The room to open in the waiting area.
  final String? roomId;

  @override
  State<WatchRoomsScreen> createState() => _WatchRoomsScreenState();
}

class _WatchRoomsScreenState extends State<WatchRoomsScreen> {
  final _invite = TextEditingController();
  final _inviteText = signal('');
  late final _focusId = signal<String?>(widget.roomId);

  /// The room a stream is being opened for.
  final _opening = signal<String?>(null);
  late final Query<GWatchRoomsData, GWatchRoomsVars> _rooms = Query(
    getIt<GraphqlService>().client,
    GWatchRoomsReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
  );
  late final Timer _poll;
  late final void Function() _stopFollowing;
  var _ticks = 0;

  /// Everyone waiting goes in when the host starts. Only a room seen waiting:
  /// someone opening a room that's already playing, or back after closing the
  /// player, chose not to be pulled in.
  String? _waitedIn;

  late final _list = computed<List<GWatchRoomFields>>(
    () => [
      for (final GWatchRoomFields room
          in _rooms.data.value?.watchRooms ?? const <GWatchRoomFields>[])
        if (room.state != GWatchRoomState.ENDED) room,
    ],
  );

  /// With one room open there's nothing to choose between, so landing here lands
  /// in it.
  late final _focused = computed<GWatchRoomFields?>(() {
    final list = _list.value;
    final id = _focusId.value;
    if (id != null) return list.where((room) => room.id == id).firstOrNull;
    return list.length == 1 ? list.first : null;
  });

  GraphqlService get _graphql => getIt<GraphqlService>();
  ToastService get _toasts => getIt<ToastService>();

  @override
  void initState() {
    super.initState();
    _poll = Timer.periodic(_roomPoll, (_) {
      _ticks += 1;
      // The list alone doesn't need to be this quick.
      if (_focused.value == null && _ticks.isOdd) return;
      if (_isVisible()) _rooms.refetch();
    });
    _stopFollowing = effect(_followHost);
  }

  void _followHost() {
    final room = _focused.value;
    if (room == null || _isHost(room)) return;
    if (room.state == GWatchRoomState.WAITING) {
      _waitedIn = room.id;
      return;
    }
    if (_waitedIn != room.id) return;
    final started =
        room.state == GWatchRoomState.PLAYING ||
        room.state == GWatchRoomState.PAUSED;
    if (!started || !_isVisible() || _opening.peek() != null) return;
    _waitedIn = null;
    unawaited(_open(room));
  }

  bool _isVisible() {
    if (!mounted || ModalRoute.of(context)?.isCurrent != true) return false;
    if (getIt<PlaybackService>().current.peek() != null) return false;
    final state = WidgetsBinding.instance.lifecycleState;
    return state != AppLifecycleState.paused &&
        state != AppLifecycleState.hidden;
  }

  @override
  void dispose() {
    _poll.cancel();
    _stopFollowing();
    _rooms.dispose();
    _invite.dispose();
    super.dispose();
  }

  bool _isHost(GWatchRoomFields room) =>
      room.hostUserId == getIt<SessionService>().viewer.peek()?.id;

  Future<void> _play(GWatchRoomFields room) async {
    final work = room.work;
    await getIt<PlaybackService>().playRoom(
      room: RoomRef(id: room.id, isHost: _isHost(room)),
      mediaFileId: room.mediaFileId,
      positionMs: room.positionMs,
      work: work == null ? null : WorkSummary.fromCard(work),
    );
  }

  Future<void> _open(GWatchRoomFields room) async {
    if (_opening.value != null) return;
    _opening.value = room.id;
    try {
      if (_isHost(room)) {
        await _play(room);
        return;
      }
      final joined = await _toasts.attempt(
        () => _graphql.run(GJoinWatchRoomReq((b) => b.vars.id = room.id)),
      );
      if (joined != null) await _play(joined.joinWatchRoom);
    } finally {
      _opening.value = null;
    }
  }

  Future<void> _joinFromInvite() async {
    final id = _roomId.stringMatch(_invite.text.trim());
    if (id == null) {
      _toasts.show(
        'That doesn’t look like a room link or ID.',
        kind: ToastKind.error,
      );
      return;
    }
    final joined = await _toasts.attempt(
      () => _graphql.run(GJoinWatchRoomReq((b) => b.vars.id = id)),
    );
    if (joined == null) return;
    _invite.clear();
    _inviteText.value = '';
    _focusId.value = id;
    _rooms.refetch();
  }

  Future<void> _end(GWatchRoomFields room) async {
    final confirmed = await confirmAction(
      title: 'End this watch room?',
      message: 'Everyone in it will be disconnected.',
      confirmLabel: 'End room',
      danger: true,
    );
    if (!confirmed) return;
    final data = await _toasts.attempt(
      () => _graphql.run(
        GControlWatchRoomReq(
          (b) => b.vars
            ..id = room.id
            ..action = GWatchRoomAction.STOP
            ..position = room.positionMs,
        ),
      ),
      success: 'Room ended',
    );
    if (data == null) return;
    _focusId.value = null;
    _rooms.refetch();
  }

  Future<void> _leave(GWatchRoomFields room) async {
    final data = await _toasts.attempt(
      () => _graphql.run(GLeaveWatchRoomReq((b) => b.vars.id = room.id)),
      success: 'You left the room',
    );
    if (data == null) return;
    _focusId.value = null;
    _rooms.refetch();
  }

  Future<void> _copyInvite(GWatchRoomFields room) async {
    final link = getIt<TokenStore>().absolute('/watch-rooms?room=${room.id}');
    await Clipboard.setData(ClipboardData(text: link));
    _toasts.show('Invite link copied', kind: ToastKind.success);
  }

  Widget _joinForm() => Align(
    alignment: Alignment.centerLeft,
    child: ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 520),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Same frame, different couches. Start a room from any title’s menu, or join with a link someone sent you.',
            style: NfType.bodyMuted,
          ),
          const SizedBox(height: 14),
          Row(
            children: [
              Expanded(
                child: NfField(
                  controller: _invite,
                  hint: 'Paste an invite link or room ID',
                  autocorrect: false,
                  keyboardType: TextInputType.url,
                  textInputAction: TextInputAction.go,
                  onChanged: (value) => _inviteText.value = value,
                  onSubmitted: (_) => unawaited(_joinFromInvite()),
                ),
              ),
              const SizedBox(width: 8),
              SignalBuilder(
                builder: (context) => NfButton(
                  label: 'Join',
                  onPressed: _inviteText.value.trim().isEmpty
                      ? null
                      : () => unawaited(_joinFromInvite()),
                ),
              ),
            ],
          ),
        ],
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return Scaffold(
      appBar: const NfPageBar(title: 'Watch together'),
      body: RefreshIndicator(
        onRefresh: () async => _rooms.refetch(),
        child: CustomScrollView(
          slivers: [
            SliverPadding(
              padding: EdgeInsets.fromLTRB(gutter, 4, gutter, 18),
              sliver: SliverToBoxAdapter(child: _joinForm()),
            ),
            SignalBuilder(
              builder: (context) {
                if (_rooms.waiting.value) {
                  return SliverPadding(
                    padding: EdgeInsets.fromLTRB(gutter, 0, gutter, 24),
                    sliver: const SliverToBoxAdapter(
                      child: NfSkeletonRows(count: 3),
                    ),
                  );
                }
                if (_rooms.data.value == null) {
                  return SliverFillRemaining(
                    hasScrollBody: false,
                    child: NfState.error(
                      title: 'Rooms are unavailable',
                      message: _rooms.error.value?.toString(),
                      retry: _rooms.refetch,
                    ),
                  );
                }
                final list = _list.value;
                final focused = _focused.value;
                final others = [
                  for (final room in list)
                    if (room.id != focused?.id) room,
                ];
                if (list.isEmpty) {
                  return const SliverFillRemaining(
                    hasScrollBody: false,
                    child: NfState(
                      icon: PhosphorIconsRegular.broadcast,
                      title: 'No rooms open',
                      message: 'Open a title, choose ••• → Start a watch room, and share the invite link.',
                    ),
                  );
                }
                return SliverMainAxisGroup(
                  slivers: [
                    if (focused != null)
                      SliverPadding(
                        padding: EdgeInsets.fromLTRB(gutter, 0, gutter, 24),
                        sliver: SliverToBoxAdapter(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              if (_focusId.value != null && list.length > 1)
                                Padding(
                                  padding: const EdgeInsets.only(bottom: 10),
                                  child: NfButton(
                                    label: 'All rooms',
                                    style: NfButtonStyle.ghost,
                                    size: NfButtonSize.small,
                                    icon: const Icon(
                                      PhosphorIconsRegular.arrowLeft,
                                    ),
                                    onPressed: () => _focusId.value = null,
                                  ),
                                ),
                              _Lobby(
                                room: focused,
                                isHost: _isHost(focused),
                                viewerId:
                                    getIt<SessionService>().viewer.value?.id,
                                opening: _opening.value == focused.id,
                                onStart: () => unawaited(_open(focused)),
                                onCopy: () => unawaited(_copyInvite(focused)),
                                onClose: () => unawaited(
                                  _isHost(focused)
                                      ? _end(focused)
                                      : _leave(focused),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ),
                    if (others.isNotEmpty) ...[
                      SliverPadding(
                        padding: EdgeInsets.fromLTRB(gutter, 0, gutter, 10),
                        sliver: SliverToBoxAdapter(
                          child: NfSectionHead(
                            focused != null ? 'Other rooms' : 'Open rooms',
                          ),
                        ),
                      ),
                      SliverPadding(
                        padding: EdgeInsets.fromLTRB(gutter, 0, gutter, 32),
                        sliver: SliverGrid.builder(
                          gridDelegate:
                              SliverGridDelegateWithMaxCrossAxisExtent(
                                maxCrossAxisExtent: 420,
                                crossAxisSpacing: 14,
                                mainAxisSpacing: 14,
                                mainAxisExtent: _RoomCard.heightFor(context),
                              ),
                          itemCount: others.length,
                          itemBuilder: (_, index) {
                            final room = others[index];
                            final host = _isHost(room);
                            return _RoomCard(
                              room: room,
                              isHost: host,
                              onOpen: () => _focusId.value = room.id,
                              onCopy: () => unawaited(_copyInvite(room)),
                              onClose: () =>
                                  unawaited(host ? _end(room) : _leave(room)),
                            );
                          },
                        ),
                      ),
                    ],
                  ],
                );
              },
            ),
          ],
        ),
      ),
    );
  }
}

String _hostName(GWatchRoomFields room) =>
    room.members
        .where((member) => member.userId == room.hostUserId)
        .firstOrNull
        ?.name ??
    'the host';

(Color, String) _stateOf(GWatchRoomFields room) => switch (room.state) {
  GWatchRoomState.PLAYING => (NfColors.danger, 'Playing'),
  GWatchRoomState.PAUSED => (NfColors.warning, 'Paused'),
  GWatchRoomState.WAITING => (NfColors.accent, 'Waiting'),
  _ => (NfColors.text3, titleCase(room.state.name)),
};

class _Lobby extends StatelessWidget {
  const _Lobby({
    required this.room,
    required this.isHost,
    required this.viewerId,
    required this.opening,
    required this.onStart,
    required this.onCopy,
    required this.onClose,
  });

  final GWatchRoomFields room;
  final bool isHost;
  final String? viewerId;
  final bool opening;
  final VoidCallback onStart;
  final VoidCallback onCopy;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final stage = _Stage(
        room: room,
        isHost: isHost,
        opening: opening,
        wide: constraints.maxWidth >= 860,
        onStart: onStart,
        onClose: onClose,
      );
      final people = _People(
        room: room,
        isHost: isHost,
        viewerId: viewerId,
        fill: constraints.maxWidth >= 860,
        onCopy: onCopy,
      );
      if (constraints.maxWidth < 860) {
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [stage, const SizedBox(height: 16), people],
        );
      }
      return IntrinsicHeight(
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Expanded(child: stage),
            const SizedBox(width: 16),
            SizedBox(width: 340, child: people),
          ],
        ),
      );
    },
  );
}

class _Stage extends StatelessWidget {
  const _Stage({
    required this.room,
    required this.isHost,
    required this.opening,
    required this.wide,
    required this.onStart,
    required this.onClose,
  });

  final GWatchRoomFields room;
  final bool isHost;
  final bool opening;
  final bool wide;
  final VoidCallback onStart;
  final VoidCallback onClose;

  @override
  Widget build(BuildContext context) {
    final card = room.work;
    final work = card == null ? null : WorkSummary.fromCard(card);
    final (dot, stateLabel) = _stateOf(room);
    final waiting = room.state == GWatchRoomState.WAITING;
    final host = _hostName(room);
    return NfPanel(
      padding: EdgeInsets.zero,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          AspectRatio(
            aspectRatio: wide ? 21 / 9 : 16 / 9,
            child: work != null
                ? Artwork(work: work, kind: ArtKind.backdrop, radius: 0)
                : const ColoredBox(
                    color: NfColors.surface2,
                    child: Icon(
                      PhosphorIconsRegular.broadcast,
                      size: 36,
                      color: NfColors.text3,
                    ),
                  ),
          ),
          Padding(
            padding: EdgeInsets.all(wide ? 28 : 18),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Icon(PhosphorIconsFill.circle, size: 8, color: dot),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        '${isHost ? 'You’re hosting' : 'Hosted by $host'} · $stateLabel'
                            .toUpperCase(),
                        style: NfType.eyebrow,
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 10),
                Text(
                  work?.heading ?? 'Watch room',
                  style: NfType.sectionTitle.copyWith(fontSize: wide ? 34 : 24),
                ),
                const SizedBox(height: 8),
                Text(
                  waiting
                      ? (isHost
                            ? 'Everyone goes in together when you start.'
                            : 'Waiting for $host to start.')
                      : '$stateLabel at ${timecode(room.positionMs)}',
                  style: NfType.bodyMuted,
                ),
                const SizedBox(height: 14),
                Wrap(
                  spacing: 10,
                  runSpacing: 8,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                    if (isHost || !waiting)
                      NfButton(
                        label: isHost ? 'Start watching' : 'Join stream',
                        style: NfButtonStyle.primary,
                        size: NfButtonSize.large,
                        icon: const Icon(PhosphorIconsFill.play),
                        busy: opening,
                        onPressed: onStart,
                      )
                    else
                      Container(
                        height: 48,
                        padding: const EdgeInsets.symmetric(horizontal: 18),
                        decoration: BoxDecoration(
                          color: NfColors.surface2,
                          borderRadius: BorderRadius.circular(NfRadii.pill),
                        ),
                        child: Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            const SizedBox.square(
                              dimension: 16,
                              child: CircularProgressIndicator(strokeWidth: 2),
                            ),
                            const SizedBox(width: 10),
                            Text(
                              'Waiting for $host',
                              style: nfText(
                                15,
                                weight: 600,
                                color: NfColors.text2,
                              ),
                            ),
                          ],
                        ),
                      ),
                    NfButton(
                      label: isHost ? 'End room' : 'Leave room',
                      style: NfButtonStyle.ghost,
                      size: NfButtonSize.large,
                      icon: Icon(
                        isHost
                            ? PhosphorIconsRegular.x
                            : PhosphorIconsRegular.signOut,
                      ),
                      onPressed: onClose,
                    ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _People extends StatelessWidget {
  const _People({
    required this.room,
    required this.isHost,
    required this.viewerId,
    required this.fill,
    required this.onCopy,
  });

  /// Beside the stage, this matches its height and keeps the link at the bottom.
  final bool fill;

  final GWatchRoomFields room;
  final bool isHost;
  final String? viewerId;
  final VoidCallback onCopy;

  @override
  Widget build(BuildContext context) {
    final members = room.members.toList();
    return NfPanel(
      padding: const EdgeInsets.all(18),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              const Icon(
                PhosphorIconsRegular.usersThree,
                size: 18,
                color: NfColors.text2,
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Text('In this room', style: nfText(15, weight: 650)),
              ),
              Text('${members.length}', style: NfType.caption),
            ],
          ),
          const SizedBox(height: 12),
          for (final member in members)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 5),
              child: Row(
                children: [
                  _Avatar(name: member.name ?? 'Someone'),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          member.name ?? 'Someone',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: nfText(14.5, weight: 600),
                        ),
                        Text(
                          '${member.userId == room.hostUserId ? 'Opened' : 'Joined'} ${ago(member.joinedAt)}',
                          style: NfType.caption,
                        ),
                      ],
                    ),
                  ),
                  if (member.userId == viewerId) ...[
                    const SizedBox(width: 6),
                    const NfBadge('You'),
                  ],
                  if (member.userId == room.hostUserId) ...[
                    const SizedBox(width: 6),
                    const NfBadge(
                      'Host',
                      color: NfColors.accent,
                      background: NfColors.accentSoft,
                    ),
                  ],
                ],
              ),
            ),
          if (members.length < 2)
            Padding(
              padding: const EdgeInsets.only(top: 6),
              child: Text(
                isHost
                    ? 'Nobody else yet. Send them the link below.'
                    : 'Just you so far.',
                style: NfType.small,
              ),
            ),
          const SizedBox(height: 14),
          if (fill) const Spacer(),
          NfButton(
            label: 'Copy invite link',
            style: NfButtonStyle.glass,
            icon: const Icon(PhosphorIconsRegular.link),
            onPressed: onCopy,
          ),
        ],
      ),
    );
  }
}

class _Avatar extends StatelessWidget {
  const _Avatar({required this.name});
  final String name;

  @override
  Widget build(BuildContext context) => Container(
    width: 34,
    height: 34,
    alignment: Alignment.center,
    decoration: const BoxDecoration(
      shape: BoxShape.circle,
      gradient: LinearGradient(
        begin: Alignment.topLeft,
        end: Alignment.bottomRight,
        colors: [NfColors.accent, Color(0xFF8DE2C5)],
      ),
    ),
    child: Text(
      (name.length > 2 ? name.substring(0, 2) : name).toUpperCase(),
      style: nfText(12, weight: 700, color: NfColors.accentInk),
    ),
  );
}

class _RoomCard extends StatelessWidget {
  const _RoomCard({
    required this.room,
    required this.isHost,
    required this.onOpen,
    required this.onCopy,
    required this.onClose,
  });

  final GWatchRoomFields room;
  final bool isHost;
  final VoidCallback onOpen;
  final VoidCallback onCopy;
  final VoidCallback onClose;

  /// The picture's a banner rather than the width-driven 16:9 on the web, so a
  /// grid of rooms keeps one height however wide its columns come out.
  static const _pictureHeight = 132.0;
  static double heightFor(BuildContext context) =>
      _pictureHeight + MediaQuery.textScalerOf(context).scale(142);

  @override
  Widget build(BuildContext context) {
    final card = room.work;
    final work = card == null ? null : WorkSummary.fromCard(card);
    final (dot, stateLabel) = _stateOf(room);
    return NfPanel(
      padding: EdgeInsets.zero,
      onTap: onOpen,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          SizedBox(
            height: _pictureHeight,
            child: Stack(
              fit: StackFit.expand,
              children: [
                if (work != null)
                  Artwork(work: work, kind: ArtKind.thumb, radius: 0)
                else
                  const ColoredBox(
                    color: NfColors.surface2,
                    child: Icon(
                      PhosphorIconsRegular.broadcast,
                      size: 30,
                      color: NfColors.text3,
                    ),
                  ),
                Positioned(
                  left: 10,
                  top: 10,
                  child: Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 10,
                      vertical: 5,
                    ),
                    decoration: BoxDecoration(
                      color: NfColors.bgOverlay,
                      borderRadius: BorderRadius.circular(NfRadii.pill),
                    ),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(PhosphorIconsFill.circle, size: 8, color: dot),
                        const SizedBox(width: 7),
                        Text(stateLabel, style: nfText(12, weight: 650)),
                      ],
                    ),
                  ),
                ),
              ],
            ),
          ),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.fromLTRB(14, 12, 8, 10),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    work?.heading ?? 'Untitled room',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: nfText(15, weight: 600),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    [
                      if (isHost)
                        'Hosted by you'
                      else
                        'Hosted by ${_hostName(room)}',
                      '${room.members.length} in the room',
                    ].join(' · '),
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: NfType.caption,
                  ),
                  const Spacer(),
                  Row(
                    children: [
                      Expanded(
                        child: Align(
                          alignment: Alignment.centerLeft,
                          child: NfButton(
                            label: 'Open room',
                            style: NfButtonStyle.primary,
                            size: NfButtonSize.small,
                            onPressed: onOpen,
                          ),
                        ),
                      ),
                      NfIconButton(
                        icon: PhosphorIconsRegular.link,
                        label: 'Copy invite link',
                        size: 40,
                        iconSize: 18,
                        color: NfColors.text2,
                        onPressed: onCopy,
                      ),
                      NfIconButton(
                        icon: isHost
                            ? PhosphorIconsRegular.x
                            : PhosphorIconsRegular.signOut,
                        label: isHost ? 'End room' : 'Leave room',
                        size: 40,
                        iconSize: 18,
                        color: NfColors.text2,
                        onPressed: onClose,
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
