// SPDX-License-Identifier: AGPL-3.0-only
//
// Cast devices on the network, as they announce themselves over mDNS
// (`_googlecast._tcp`). Bonsoir asks each platform's own service browser
// (NsdManager, Bonjour, Avahi, DNS-SD on Windows), so iOS needs only
// `NSBonjourServices` rather than the multicast entitlement a raw mDNS socket
// would.

import 'dart:async';

import 'package:bonsoir/bonsoir.dart';
import 'package:flutter/foundation.dart';
import 'package:signals_flutter/signals_flutter.dart';

const _type = '_googlecast._tcp';

/// One Cast device, from its announcement's TXT record.
class CastReceiverInfo {
  const CastReceiverInfo({
    required this.id,
    required this.name,
    required this.model,
    required this.host,
    required this.port,
    this.service = '',
  });

  /// The device's own id (`id`), stable across addresses.
  final String id;

  /// What its owner named it (`fn`): "Living Room TV".
  final String name;

  /// `md`: "Chromecast with Google TV", or a TV's own model.
  final String model;
  final String host;
  final int port;

  /// The mDNS instance name, which a loss names it by.
  final String service;
}

/// Whether an announcement is a device that shows video: `ca` is a bit set,
/// and speakers and speaker groups leave out video out (1).
@visibleForTesting
bool showsVideo(Map<String, String> txt) {
  final caps = int.tryParse(txt['ca'] ?? '');
  return caps == null || caps & 1 == 1;
}

/// IPv4 where there is one: Avahi lists IPv6 link-local addresses that a
/// socket can't reach without a zone.
String? _address(List<String> addresses) =>
    addresses.where((a) => !a.contains(':')).firstOrNull ??
    addresses.firstOrNull;

class CastDiscovery {
  final devices = signal<List<CastReceiverInfo>>(const []);
  BonsoirDiscovery? _discovery;
  StreamSubscription<BonsoirDiscoveryEvent>? _events;

  Future<void> start() async {
    if (_discovery != null) return;
    final discovery = BonsoirDiscovery(type: _type, printLogs: false);
    _discovery = discovery;
    try {
      await discovery.initialize();
      _events = discovery.eventStream?.listen((event) {
        switch (event) {
          case BonsoirDiscoveryServiceFoundEvent(:final service):
            discovery.serviceResolver.resolveService(service);
          case BonsoirDiscoveryServiceResolvedEvent(:final service) ||
              BonsoirDiscoveryServiceUpdatedEvent(:final service):
            _found(service);
          case BonsoirDiscoveryServiceLostEvent(:final service):
            devices.value = [
              for (final device in devices.value)
                if (device.service != service.name) device,
            ];
          default:
        }
      });
      await discovery.start();
    } on Object catch (error) {
      // No mDNS here (Avahi isn't running, say): Cast just isn't offered.
      debugPrint('Cast discovery unavailable: $error');
      await stop();
    }
  }

  void _found(BonsoirService service) {
    final txt = service.attributes;
    final host = _address(service.hostAddresses) ?? service.hostname;
    final id = txt['id'];
    if (host == null || id == null || !showsVideo(txt)) return;
    final device = CastReceiverInfo(
      id: id,
      name: txt['fn'] ?? service.name,
      model: txt['md'] ?? '',
      host: host,
      port: service.port,
      service: service.name,
    );
    devices.value = [
      for (final known in devices.value)
        if (known.id != id) known,
      device,
    ]..sort((a, b) => a.name.compareTo(b.name));
  }

  Future<void> stop() async {
    final discovery = _discovery;
    _discovery = null;
    await _events?.cancel();
    _events = null;
    devices.value = const [];
    if (discovery != null && !discovery.isStopped) {
      try {
        await discovery.stop();
      } on Object {
        // Already gone with the platform's browser.
      }
    }
  }
}
