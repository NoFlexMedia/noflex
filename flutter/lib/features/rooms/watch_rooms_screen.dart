// SPDX-License-Identifier: AGPL-3.0-only
//
// Rooms are only listed to their members, so someone with an invite joins by
// pasting it here. A room's host opens it as host; everyone else joins and
// follows.

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

/// There's no subscription for the room list itself.
const _roomPoll = Duration(seconds: 4);

final _roomId = RegExp(
  r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}',
  caseSensitive: false,
);

class WatchRoomsScreen extends StatefulWidget {
  const WatchRoomsScreen({super.key});

  @override
  State<WatchRoomsScreen> createState() => _WatchRoomsScreenState();
}

class _WatchRoomsScreenState extends State<WatchRoomsScreen> {
  final _invite = TextEditingController();
  final _inviteText = signal('');

  /// The room a stream is being opened for.
  final _opening = signal<String?>(null);
  late final Query<GWatchRoomsData, GWatchRoomsVars> _rooms = Query(
    getIt<GraphqlService>().client,
    GWatchRoomsReq((b) => b..fetchPolicy = FetchPolicy.NetworkOnly),
  );
  late final Timer _poll;

  GraphqlService get _graphql => getIt<GraphqlService>();
  ToastService get _toasts => getIt<ToastService>();

  @override
  void initState() {
    super.initState();
    _poll = Timer.periodic(_roomPoll, (_) {
      if (_isVisible()) _rooms.refetch();
    });
  }

  bool _isVisible() {
    if (!mounted || ModalRoute.of(context)?.isCurrent != true) return false;
    if (getIt<PlaybackService>().current.value != null) return false;
    final state = WidgetsBinding.instance.lifecycleState;
    return state != AppLifecycleState.paused &&
        state != AppLifecycleState.hidden;
  }

  @override
  void dispose() {
    _poll.cancel();
    _rooms.dispose();
    _invite.dispose();
    super.dispose();
  }

  bool _isHost(GWatchRoomFields room) =>
      room.hostUserId == getIt<SessionService>().viewer.value?.id;

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
    _rooms.refetch();
    await _play(joined.joinWatchRoom);
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
    if (data != null) _rooms.refetch();
  }

  Future<void> _leave(GWatchRoomFields room) async {
    final data = await _toasts.attempt(
      () => _graphql.run(GLeaveWatchRoomReq((b) => b.vars.id = room.id)),
      success: 'You left the room',
    );
    if (data != null) _rooms.refetch();
  }

  Future<void> _copyInvite(GWatchRoomFields room) async {
    final link = getIt<TokenStore>().absolute('/watch-rooms?room=${room.id}');
    await Clipboard.setData(ClipboardData(text: link));
    _toasts.show('Invite link copied', kind: ToastKind.success);
  }

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
              padding: EdgeInsets.fromLTRB(gutter, 4, gutter, 16),
              sliver: SliverToBoxAdapter(
                child: Align(
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
                                hint: 'Invite link or room ID',
                                autocorrect: false,
                                keyboardType: TextInputType.url,
                                textInputAction: TextInputAction.go,
                                onChanged: (value) => _inviteText.value = value,
                                onSubmitted: (_) =>
                                    unawaited(_joinFromInvite()),
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
                ),
              ),
            ),
            SignalBuilder(
              builder: (context) {
                if (_rooms.waiting.value) {
                  return const SliverFillRemaining(
                    hasScrollBody: false,
                    child: Center(child: CircularProgressIndicator()),
                  );
                }
                final data = _rooms.data.value;
                if (data == null) {
                  return SliverFillRemaining(
                    hasScrollBody: false,
                    child: NfState.error(
                      title: 'Rooms are unavailable',
                      message: _rooms.error.value?.toString(),
                      retry: _rooms.refetch,
                    ),
                  );
                }
                final rooms = data.watchRooms;
                if (rooms.isEmpty) {
                  return const SliverFillRemaining(
                    hasScrollBody: false,
                    child: NfState(
                      icon: PhosphorIconsRegular.broadcast,
                      title: 'No rooms open',
                      message: 'Open a title, choose ••• → Start a watch room, and share the invite link.',
                    ),
                  );
                }
                return SliverPadding(
                  padding: EdgeInsets.fromLTRB(gutter, 0, gutter, 32),
                  sliver: SliverGrid.builder(
                    gridDelegate: SliverGridDelegateWithMaxCrossAxisExtent(
                      maxCrossAxisExtent: 420,
                      crossAxisSpacing: 14,
                      mainAxisSpacing: 14,
                      mainAxisExtent: _RoomCard.heightFor(context),
                    ),
                    itemCount: rooms.length,
                    itemBuilder: (_, index) {
                      final room = rooms[index];
                      final host = _isHost(room);
                      return _RoomCard(
                        room: room,
                        isHost: host,
                        opening: _opening.value == room.id,
                        onOpen: () => unawaited(_open(room)),
                        onCopy: () => unawaited(_copyInvite(room)),
                        onClose: () =>
                            unawaited(host ? _end(room) : _leave(room)),
                      );
                    },
                  ),
                );
              },
            ),
          ],
        ),
      ),
    );
  }
}

class _RoomCard extends StatelessWidget {
  const _RoomCard({
    required this.room,
    required this.isHost,
    required this.opening,
    required this.onOpen,
    required this.onCopy,
    required this.onClose,
  });

  final GWatchRoomFields room;
  final bool isHost;
  final bool opening;
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
    final ended = room.state == GWatchRoomState.ENDED;
    final (dot, stateLabel) = switch (room.state) {
      GWatchRoomState.PLAYING => (NfColors.danger, 'Playing'),
      GWatchRoomState.PAUSED => (NfColors.warning, 'Paused'),
      GWatchRoomState.WAITING => (NfColors.accent, 'Waiting'),
      _ => (NfColors.text3, titleCase(room.state.name)),
    };
    return NfPanel(
      padding: EdgeInsets.zero,
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
                      if (isHost) 'Hosted by you' else 'Hosted by a friend',
                      if (room.state == GWatchRoomState.WAITING)
                        'waiting for the host to press play'
                      else if (!ended)
                        'at ${timecode(room.positionMs)}',
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
                            label: isHost ? 'Start watching' : 'Join',
                            style: NfButtonStyle.primary,
                            size: NfButtonSize.small,
                            icon: const Icon(PhosphorIconsFill.play),
                            busy: opening,
                            onPressed: ended ? null : onOpen,
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
