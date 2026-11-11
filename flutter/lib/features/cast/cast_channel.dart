// SPDX-License-Identifier: AGPL-3.0-only
//
// One TLS connection to a Cast device, speaking CASTV2 the way Google's sender
// SDKs do: a virtual connection to the device (`receiver-0`) and one to each
// app it runs, a heartbeat both ways, and JSON requests matched to their
// replies. Cast devices present a self-signed certificate, so it's accepted
// unchecked, as every sender without Google's SDK does; the device isn't
// authenticated (cast/AGENTS.md).

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'cast_message.dart';

const platformReceiver = 'receiver-0';
const connectionNamespace = 'urn:x-cast:com.google.cast.tp.connection';
const heartbeatNamespace = 'urn:x-cast:com.google.cast.tp.heartbeat';

/// A message the device sent this sender, its payload parsed.
class CastEvent {
  const CastEvent(this.source, this.namespace, this.data);
  final String source;
  final String namespace;
  final Map<String, dynamic> data;
  String? get type => data['type'] as String?;
}

class CastChannel {
  CastChannel._(this._socket) {
    final reader = CastFrameReader();
    _socket.listen(
      (chunk) {
        for (final message in reader.add(chunk)) {
          _receive(message);
        }
      },
      onError: (Object _) => close(),
      onDone: close,
    );
    _beat = Timer.periodic(const Duration(seconds: 5), (_) {
      if (DateTime.now().difference(_heard) > const Duration(seconds: 16)) {
        close();
      } else {
        send(heartbeatNamespace, {'type': 'PING'});
      }
    });
  }

  /// This sender's id on the device, unique per channel.
  final sender = 'sender-noflex-${DateTime.now().microsecondsSinceEpoch}';
  final Socket _socket;
  late final Timer _beat;
  DateTime _heard = DateTime.now();
  final _events = StreamController<CastEvent>.broadcast();
  final _replies = <String, Completer<CastEvent>>{};
  final _closed = Completer<void>();
  var _nextRequest = 0;

  static Future<CastChannel> connect(String host, int port) async {
    final socket = await SecureSocket.connect(
      host,
      port,
      onBadCertificate: (_) => true,
      timeout: const Duration(seconds: 5),
    );
    return CastChannel._(socket)..open(platformReceiver);
  }

  /// What the device says to this sender, or to every sender.
  Stream<CastEvent> get events => _events.stream;

  /// Completes when the connection goes, whichever end ends it.
  Future<void> get closed => _closed.future;
  bool get isClosed => _closed.isCompleted;

  /// Open the virtual connection to [destination]: the device, or an app's transport.
  void open(String destination) => send(connectionNamespace, {
    'type': 'CONNECT',
    'origin': <String, Object>{},
  }, destination: destination);

  void send(
    String namespace,
    Map<String, Object?> payload, {
    String destination = platformReceiver,
  }) {
    if (isClosed) return;
    _socket.add(
      CastMessage(
        source: sender,
        destination: destination,
        namespace: namespace,
        payload: jsonEncode(payload),
      ).encode(),
    );
  }

  /// Send [payload] with a fresh id under [idKey] and wait for the reply that
  /// carries it back: `requestId` for Google's namespaces.
  Future<CastEvent> request(
    String namespace,
    Map<String, Object?> payload, {
    String destination = platformReceiver,
    String idKey = 'requestId',
    Duration timeout = const Duration(seconds: 10),
  }) {
    final id = ++_nextRequest;
    final key = '$namespace/$id';
    final reply = Completer<CastEvent>();
    _replies[key] = reply;
    send(namespace, {...payload, idKey: id}, destination: destination);
    return reply.future
        .timeout(
          timeout,
          onTimeout: () =>
              throw TimeoutException('The Cast device didn’t answer.'),
        )
        .whenComplete(() => _replies.remove(key));
  }

  void _receive(CastMessage message) {
    _heard = DateTime.now();
    if (message.destination != sender && message.destination != '*') return;
    final Map<String, dynamic> data;
    try {
      data = jsonDecode(message.payload) as Map<String, dynamic>;
    } on FormatException {
      return;
    }
    if (message.namespace == heartbeatNamespace) {
      if (data['type'] == 'PING') {
        send(heartbeatNamespace, {'type': 'PONG'}, destination: message.source);
      }
      return;
    }
    final event = CastEvent(message.source, message.namespace, data);
    if (message.namespace == connectionNamespace && event.type == 'CLOSE') {
      // The device closed the virtual connection; to itself, that's everything.
      if (message.source == platformReceiver) close();
    }
    final id = data['requestId'] ?? data['id'];
    final reply = id is int ? _replies['${message.namespace}/$id'] : null;
    if (reply != null && !reply.isCompleted) reply.complete(event);
    _events.add(event);
  }

  Future<void> close() async {
    if (isClosed) return;
    _closed.complete();
    _beat.cancel();
    for (final reply in _replies.values) {
      if (!reply.isCompleted) {
        reply.completeError(
          const SocketException('The Cast device went away.'),
        );
      }
    }
    await _events.close();
    // Sent first, so a last STOP reaches the device rather than a reset.
    try {
      await _socket.flush();
      await _socket.close();
    } on Object {
      _socket.destroy();
    }
  }
}
