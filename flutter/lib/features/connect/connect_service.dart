// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex Connect from the controlling side: the viewer's other devices, which
// of them are listening, what they're playing, and the choice of where a Play
// goes. The state is whatever the server last said; a device's position is
// advanced here between its reports.

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/cast_to_device.req.gql.dart';
import '../../graphql/queries/__generated__/connect_device_updates.req.gql.dart';
import '../../graphql/queries/__generated__/control_device.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/connect_device_fields.data.gql.dart';
import '../../models/work.dart';
import '../../services/device_identity.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';

/// What another device is playing, as it last said.
class RemotePlayback {
  const RemotePlayback({
    required this.state,
    required this.positionMs,
    this.durationMs,
    this.work,
  });

  final GDevicePlayState state;
  final int positionMs;
  final int? durationMs;
  final WorkSummary? work;

  RemotePlayback copyWith({GDevicePlayState? state, int? positionMs}) =>
      RemotePlayback(
        state: state ?? this.state,
        positionMs: positionMs ?? this.positionMs,
        durationMs: durationMs,
        work: work,
      );
}

/// How a device is steered: through the server, or over Cast from here.
enum RemoteVia { connect, cast }

/// Somewhere to play other than here: one of the viewer's devices as the
/// server last described it, or a Cast device this app found (`CastService`).
class RemoteDevice {
  const RemoteDevice({
    required this.id,
    required this.name,
    required this.heardAt,
    this.client,
    this.online = true,
    this.playback,
    this.via = RemoteVia.connect,
  });

  factory RemoteDevice.fromConnect(
    GConnectDeviceFields data,
    DateTime heardAt,
  ) {
    final playback = data.playback;
    final work = playback?.work;
    return RemoteDevice(
      id: data.device.id,
      name: data.device.name,
      client: data.device.client,
      online: data.online,
      heardAt: heardAt,
      playback: playback == null
          ? null
          : RemotePlayback(
              state: playback.state,
              positionMs: playback.positionMs,
              durationMs: playback.durationMs,
              work: work == null ? null : WorkSummary.fromCard(work),
            ),
    );
  }

  final String id;
  final String name;
  final String? client;
  final bool online;
  final RemotePlayback? playback;
  final RemoteVia via;

  /// When this was heard, by this clock: [ConnectService.position] advances from it.
  final DateTime heardAt;

  GDevicePlayState? get state => playback?.state;
  int get positionMs => playback?.positionMs ?? 0;
  bool get playing => state == GDevicePlayState.PLAYING;
  bool get buffering => state == GDevicePlayState.BUFFERING;

  /// What a control just asked for, shown until the device's word replaces it.
  RemoteDevice expecting(RemotePlayback? playback) => RemoteDevice(
    id: id,
    name: name,
    client: client,
    online: online,
    heardAt: DateTime.now(),
    playback: playback,
    via: via,
  );
}

/// Steering for devices the server doesn't know: Cast.
abstract interface class RemoteRoute {
  Future<bool> play(
    RemoteDevice device,
    WorkSummary work, {
    String? editionId,
    String? mediaFileId,
    int? positionMs,
  });

  Future<void> control(
    RemoteDevice device,
    GDeviceControl action, {
    int? positionMs,
  });
}

class ConnectService {
  ConnectService({
    required SessionService session,
    required this._graphql,
    required this._identity,
    required this._toasts,
  }) {
    _stopWatchingSession = effect(() {
      if (session.phase.value == AppPhase.app) {
        _start();
      } else {
        _stop();
      }
    });
  }

  final GraphqlService _graphql;
  final DeviceIdentity _identity;
  final ToastService _toasts;
  late final void Function() _stopWatchingSession;
  StreamSubscription<dynamic>? _updates;
  Timer? _retry;
  Timer? _tick;

  /// Every device of the viewer's, this one excluded.
  final devices = signal<List<RemoteDevice>>(const []);

  /// Devices found here rather than through the server, and how to steer them.
  final external = signal<List<RemoteDevice>>(const []);
  RemoteRoute? castRoute;

  /// Where Play goes; null plays here.
  final targetId = signal<String?>(null);

  /// Ticks while something plays remotely, so a position advances smoothly.
  final now = signal(DateTime.now());

  /// Devices that can be played on right now.
  late final available = computed(
    () => [
      for (final device in devices.value)
        if (device.online) device,
      ...external.value,
    ],
  );

  late final target = computed(() {
    final id = targetId.value;
    if (id == null) return null;
    for (final device in available.value) {
      if (device.id == id) return device;
    }
    return null;
  });

  /// What the now-playing bar shows: the chosen device if it has something,
  /// else whichever does.
  late final active = computed(() {
    final chosen = target.value;
    if (chosen?.playback != null) return chosen;
    for (final device in available.value) {
      if (device.playback != null) return device;
    }
    return null;
  });

  /// Where [device] is up to in milliseconds, advanced since its last report
  /// while it plays.
  int position(RemoteDevice device) {
    final playback = device.playback;
    if (playback == null) return 0;
    final elapsed = device.playing
        ? now.value.difference(device.heardAt).inMilliseconds
        : 0;
    final at = device.positionMs + elapsed;
    final length = playback.durationMs;
    return (length == null ? at : at.clamp(0, length)).clamp(0, 1 << 40);
  }

  void setTarget(String? id) => targetId.value = id;

  void _start() {
    if (_updates != null) return;
    _listen();
    _tick = Timer.periodic(const Duration(milliseconds: 500), (_) {
      if (active.peek() != null) now.value = DateTime.now();
    });
  }

  void _stop() {
    _retry?.cancel();
    _tick?.cancel();
    unawaited(_updates?.cancel());
    _updates = null;
    batch(() {
      devices.value = const [];
      targetId.value = null;
    });
  }

  void _listen() {
    _updates = _graphql.client
        .request(
          GConnectDeviceUpdatesReq(
            (b) => b..fetchPolicy = FetchPolicy.NetworkOnly,
          ),
        )
        .listen(
          (response) {
            final list = response.data?.connectDevices;
            if (list == null) return;
            final heard = DateTime.now();
            batch(() {
              devices.value = [
                for (final device in list)
                  if (device.device.id != _identity.id)
                    RemoteDevice.fromConnect(device, heard),
              ];
              if (targetId.peek() != null && target.peek() == null) {
                targetId.value = null;
              }
            });
          },
          onError: (Object _) => _reconnect(),
          onDone: _reconnect,
        );
  }

  void _reconnect() {
    if (_updates == null) return;
    unawaited(_updates?.cancel());
    _retry?.cancel();
    _retry = Timer(const Duration(seconds: 5), _listen);
  }

  void _replace(GConnectDeviceFields fresh) {
    final heard = DateTime.now();
    devices.value = [
      for (final device in devices.value)
        if (device.id == fresh.device.id)
          RemoteDevice.fromConnect(fresh, heard)
        else
          device,
    ];
  }

  void _expect(RemoteDevice device, RemotePlayback? playback) {
    devices.value = [
      for (final each in devices.value)
        if (each.id == device.id) device.expecting(playback) else each,
    ];
  }

  /// Start [work] on the chosen device, which picks the version it plays best.
  Future<bool> cast(
    WorkSummary work, {
    String? editionId,
    String? mediaFileId,
    int? positionMs,
  }) async {
    final device = target.peek();
    if (device == null) return false;
    if (device.via == RemoteVia.cast) {
      return await castRoute?.play(
            device,
            work,
            editionId: editionId,
            mediaFileId: mediaFileId,
            positionMs: positionMs,
          ) ??
          false;
    }
    final sent = await _toasts.attempt(
      () => _graphql.run(
        GCastToDeviceReq(
          (b) => b.vars
            ..device = device.id
            ..work = work.id
            ..edition = editionId
            ..media = mediaFileId
            ..position = positionMs,
        ),
      ),
    );
    if (sent != null) _replace(sent.castToDevice);
    return sent != null;
  }

  /// Steer the device that's playing. The control answers at once; the
  /// device's word follows.
  Future<void> control(
    RemoteDevice device,
    GDeviceControl action, {
    int? positionMs,
  }) async {
    if (device.via == RemoteVia.cast) {
      return castRoute?.control(device, action, positionMs: positionMs);
    }
    final playback = device.playback;
    if (action == GDeviceControl.STOP) {
      // Stopped is stopped: the bar goes now, and the device's word that it's idle only confirms it.
      _expect(device, null);
    } else if (playback != null) {
      _expect(
        device,
        playback.copyWith(
          state: switch (action) {
            GDeviceControl.PAUSE => GDevicePlayState.PAUSED,
            GDeviceControl.PLAY => GDevicePlayState.PLAYING,
            _ => playback.state,
          },
          positionMs: action == GDeviceControl.SEEK && positionMs != null
              ? positionMs
              : position(device),
        ),
      );
    }
    final sent = await _toasts.attempt(
      () => _graphql.run(
        GControlDeviceReq(
          (b) => b.vars
            ..device = device.id
            ..action = action
            ..position = positionMs,
        ),
      ),
    );
    if (sent != null) _replace(sent.controlDevice);
  }

  void dispose() {
    _stopWatchingSession();
    _stop();
  }
}
