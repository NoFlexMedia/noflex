// SPDX-License-Identifier: AGPL-3.0-only
//
// Moving along an adaptive session's ladder, which libmpv can't do itself:
// FFmpeg's HLS demuxer plays whichever variant it's given and never switches.
// The player opens one variant's playlist and this says when another would
// play better. Moving between the original and the ladder is AutoQuality's.

import 'quality.dart';

/// One variant of a multi-variant playlist.
class Variant {
  const Variant({
    required this.uri,
    required this.bandwidth,
    this.width,
    this.height,
  });

  final Uri uri;

  /// `BANDWIDTH`, in bits per second.
  final int bandwidth;
  final int? width;
  final int? height;
}

/// The variants a master playlist lists, best first. Empty for a media
/// playlist.
List<Variant> parseMaster(String body, Uri base) {
  final variants = <Variant>[];
  String? info;
  for (final raw in body.split('\n')) {
    final line = raw.trim();
    if (line.startsWith('#EXT-X-STREAM-INF:')) {
      info = line;
    } else if (info != null && line.isNotEmpty && !line.startsWith('#')) {
      final bandwidth = RegExp(r'[:,]BANDWIDTH=(\d+)').firstMatch(info);
      final size = RegExp(r'RESOLUTION=(\d+)x(\d+)').firstMatch(info);
      if (bandwidth != null) {
        variants.add(
          Variant(
            uri: base.resolve(line),
            bandwidth: int.parse(bandwidth[1]!),
            width: size == null ? null : int.parse(size[1]!),
            height: size == null ? null : int.parse(size[2]!),
          ),
        );
      }
      info = null;
    }
  }
  return variants..sort((a, b) => b.bandwidth.compareTo(a.bandwidth));
}

/// Thresholds for [LadderSteps], in milliseconds. Every move reopens the
/// stream, which a viewer sees as a short wait, so they're slower than a
/// player's own ABR would be.
abstract final class Steps {
  /// A variant fits an estimate this much over its bandwidth.
  static const fit = 1.25;

  /// How long a better variant must fit before moving up to it.
  static const upMs = 20000;

  /// The least time on a variant before moving up from it.
  static const stayMs = 45000;
}

int _wallClock() => DateTime.now().millisecondsSinceEpoch;

/// Which of [variants] (best first) to play, fed the player's stalls and
/// bandwidth estimate. Down on the stalls that would make Auto leave the
/// original; up once a better variant has fit the estimate for a while.
class LadderSteps {
  /// Starting on variant [_at].
  LadderSteps(this.variants, this._at, {this.now = _wallClock}) {
    _movedAt = now();
    _stalls = AutoQuality(now)..settle();
  }

  final List<Variant> variants;
  final int Function() now;
  int _at;
  late int _movedAt;
  late AutoQuality _stalls;
  int? _fitsSince;

  /// The variant playing.
  int get at => _at;

  /// The best variant [estimateBps] holds; the best of all with no estimate.
  static int pick(List<Variant> variants, int? estimateBps) {
    if (estimateBps == null) return 0;
    final index = variants.indexWhere(
      (v) => v.bandwidth * Steps.fit <= estimateBps,
    );
    return index < 0 ? variants.length - 1 : index;
  }

  void settle() => _stalls.settle();
  void stalled() => _stalls.stalled();
  void resumed() => _stalls.resumed();

  /// The variant to move to now, or null to stay.
  int? next(int? estimateBps) {
    final at = now();
    final best = pick(variants, estimateBps);
    if (_stalls.struggling()) {
      if (_at == variants.length - 1) return null;
      // At least one down, further if the estimate says so.
      return _moved(best > _at ? best : _at + 1, at);
    }
    if (best >= _at) {
      _fitsSince = null;
      return null;
    }
    final since = _fitsSince ??= at;
    if (at - since < Steps.upMs || at - _movedAt < Steps.stayMs) return null;
    return _moved(best, at);
  }

  int _moved(int to, int at) {
    _at = to;
    _movedAt = at;
    _fitsSince = null;
    _stalls = AutoQuality(now)..settle();
    return to;
  }
}
