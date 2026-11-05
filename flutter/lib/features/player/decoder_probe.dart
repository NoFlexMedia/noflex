// SPDX-License-Identifier: AGPL-3.0-only
//
// Measures what this device decodes in hardware by asking the real decoder:
// libmpv plays a few black frames of each codec with no video output and says
// what it used. One code path on every platform, and the same stack playback
// will use, so a driver or OS claim can't disagree with what actually happens.

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:media_kit/media_kit.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:signals_flutter/signals_flutter.dart';

import 'decoders.dart';

const _cacheKey = 'noflex.decoderProfile';
const _clipDir = 'asset:///assets/decoder-probes';

/// Smallest first, so a codec with no hardware at all costs one clip.
const _ladder = [
  (height: 1080, depth: 8, pixels: pixels1080p),
  (height: 2160, depth: 8, pixels: pixels2160p),
  (height: 1080, depth: 10, pixels: pixels1080p),
  (height: 2160, depth: 10, pixels: pixels2160p),
];

/// What software can sustain. Phones throttle long before their benchmark
/// numbers suggest, so they get 1080p; desktops get 4K.
DecodeCeiling softwareCeilingFor({
  required bool mobile,
  required int processors,
}) => mobile
    ? const DecodeCeiling(pixels1080p, 10)
    : processors >= 6
    ? const DecodeCeiling(pixels2160p, 10)
    : const DecodeCeiling(pixels1080p, 10);

class DecoderProbeService {
  final profile = signal<DecoderProfile?>(null);
  Future<DecoderProfile>? _measuring;

  DecodeCeiling get _software => softwareCeilingFor(
    mobile: Platform.isAndroid || Platform.isIOS,
    processors: Platform.numberOfProcessors,
  );

  /// The measured profile, from cache when this build on this OS has one.
  Future<DecoderProfile> ready() => _measuring ??= _load();

  Future<DecoderProfile> _load() async {
    final prefs = await SharedPreferences.getInstance();
    final key = await _fingerprint();
    final cached = prefs.getString(_cacheKey);
    if (cached != null) {
      try {
        final json = jsonDecode(cached) as Map<String, dynamic>;
        if (json['key'] == key) {
          final loaded = DecoderProfile.fromJson(json);
          profile.value = loaded;
          return loaded;
        }
      } catch (_) {
        // An unreadable cache is just a re-measure.
      }
    }
    return remeasure();
  }

  Future<DecoderProfile> remeasure() async {
    final measured = await _measure();
    profile.value = measured;
    _measuring = Future.value(measured);
    await _store(measured);
    return measured;
  }

  /// Playback ended up in software where the measurement said hardware.
  Future<void> noteSoftwareFallback(DecoderProfile corrected) async {
    profile.value = corrected;
    _measuring = Future.value(corrected);
    await _store(corrected);
  }

  Future<void> _store(DecoderProfile value) async {
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(
      _cacheKey,
      jsonEncode({...value.toJson(), 'key': await _fingerprint()}),
    );
  }

  static Future<String> _fingerprint() async {
    final info = await PackageInfo.fromPlatform();
    return '${info.version}+${info.buildNumber}/${Platform.operatingSystemVersion}';
  }

  Future<DecoderProfile> _measure() async {
    final watch = Stopwatch()..start();
    final hardware = <String, List<DecodeCeiling>>{};
    var audio = const <String>{};
    String? api;
    final player = Player(
      configuration: const PlayerConfiguration(
        vo: 'null',
        muted: true,
        title: 'NoFlex decoder probe',
      ),
    );
    try {
      final native = player.platform as NativePlayer;
      await native.setProperty('vid', 'auto');
      await native.setProperty('aid', 'no');
      await native.setProperty('sid', 'no');
      await native.setProperty('hwdec', 'auto-copy');
      await native.setProperty('hwdec-codecs', 'all');
      audio = _audioDecoders(await native.getProperty('decoder-list'));
      for (final codec in measuredCodecs) {
        for (final step in _ladder) {
          if (codec == 'h264' && step.depth == 10) continue;
          final used = await _decodeWith(
            native,
            player,
            '$codec-${step.height}-${step.depth}',
          );
          if (used == null) {
            // Nothing bigger or deeper will succeed where 1080p 8-bit failed.
            if (step.height == 1080 && step.depth == 8) break;
            continue;
          }
          api = used;
          (hardware[codec] ??= []).add(DecodeCeiling(step.pixels, step.depth));
        }
      }
    } catch (error) {
      debugPrint('decoder probe failed: $error');
    } finally {
      unawaited(player.dispose());
    }
    debugPrint(
      'decoder probe: $hardware via $api in ${watch.elapsedMilliseconds}ms',
    );
    return DecoderProfile(
      hardware: hardware,
      softwareCeiling: _software,
      audioDecoders: audio,
      hwdecApi: api,
    );
  }

  /// The hwdec libmpv settled on for a clip, or null for software or failure.
  /// Opened paused: the decoder still comes up for the first frame, and stays
  /// up. A clip left playing is over before a slow one such as Vulkan reports,
  /// and libmpv's properties empty out at the end of a file.
  Future<String?> _decodeWith(
    NativePlayer native,
    Player player,
    String clip,
  ) async {
    await player.open(Media('$_clipDir/$clip.mkv'), play: false);
    final deadline = DateTime.now().add(const Duration(seconds: 3));
    while (DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 40));
      final current = await native.getProperty('hwdec-current');
      if (current.isNotEmpty && current != 'no') return current;
      // A frame out of the decoder means it has made up its mind.
      final format = await native.getProperty('video-params/pixelformat');
      if (format.isNotEmpty) {
        final settled = await native.getProperty('hwdec-current');
        return settled.isNotEmpty && settled != 'no' ? settled : null;
      }
    }
    return null;
  }

  static Set<String> _audioDecoders(String list) {
    try {
      final decoded = jsonDecode(list) as List;
      return {
        for (final entry in decoded)
          if (entry is Map && entry['codec'] is String)
            entry['codec'] as String,
      };
    } catch (_) {
      return const {};
    }
  }
}
