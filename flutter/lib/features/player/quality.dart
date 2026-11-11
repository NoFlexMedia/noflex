// SPDX-License-Identifier: AGPL-3.0-only
//
// Stream quality: what the viewer chose, what to ask the server for, and when
// Auto leaves the original for the transcode ladder and comes back. A port of
// lib/ts/quality.ts, which is canonical: a threshold changes in both.

/// One step of a session's `qualities`.
class StreamQuality {
  const StreamQuality({
    required this.label,
    required this.width,
    required this.height,
    required this.bitrateBps,
  });

  final String label;
  final int width;
  final int height;
  final int bitrateBps;
}

enum QualityKind { original, auto, pinned }

/// The original file, Auto, or one step pinned by its bitrate.
class QualityChoice {
  const QualityChoice._(this.kind, [this.bitrateBps]);

  static const original = QualityChoice._(QualityKind.original);
  static const auto = QualityChoice._(QualityKind.auto);
  const QualityChoice.pinned(int bitrateBps)
    : this._(QualityKind.pinned, bitrateBps);

  final QualityKind kind;

  /// Set only when [kind] is pinned.
  final int? bitrateBps;

  @override
  bool operator ==(Object other) =>
      other is QualityChoice &&
      other.kind == kind &&
      other.bitrateBps == bitrateBps;

  @override
  int get hashCode => Object.hash(kind, bitrateBps);

  @override
  String toString() => encodeChoice(this);
}

/// What to ask `startPlaybackSession` for.
typedef SessionQuality = ({bool adaptive, int? maxBitrateBps});

/// [leftOriginal] is Auto after the original struggled.
SessionQuality sessionQuality(
  QualityChoice choice,
  int? originalBitrateBps, {
  bool leftOriginal = false,
}) => switch (choice.kind) {
  QualityKind.pinned => (adaptive: false, maxBitrateBps: choice.bitrateBps),
  QualityKind.original => (adaptive: false, maxBitrateBps: null),
  // Under the original's own rate, so the server encodes rather than sending it again.
  QualityKind.auto => (
    adaptive: true,
    maxBitrateBps:
        leftOriginal && originalBitrateBps != null && originalBitrateBps > 0
        ? (originalBitrateBps * 0.9).floor()
        : null,
  ),
};

const _frames = [
  ('360p', 640, 360),
  ('480p', 854, 480),
  ('720p', 1280, 720),
  ('1080p', 1920, 1080),
  ('1440p', 2560, 1440),
  ('4K', 3840, 2160),
];

/// The 16:9 frame a picture fits, as the server names its steps: 1280×544 is 720p.
String? frameLabel(int? width, int? height) {
  if (width == null || height == null || width == 0 || height == 0) {
    return null;
  }
  for (final (label, w, h) in _frames) {
    if (width * 20 <= w * 21 && height * 20 <= h * 21) return label;
  }
  return '4K';
}

String megabits(int? bps) {
  if (bps == null || bps == 0) return '';
  final value = bps / 1e6;
  return '${value >= 10 ? value.round() : value.toStringAsFixed(1)} Mb/s';
}

/// `1080p · 8.0 Mb/s`
String qualityText(StreamQuality quality) =>
    '${quality.label} · ${megabits(quality.bitrateBps)}';

/// A choice as it's kept between plays: `original`, `auto` or a bitrate.
String encodeChoice(QualityChoice choice) => choice.kind == QualityKind.pinned
    ? '${choice.bitrateBps}'
    : choice.kind.name;

QualityChoice decodeChoice(String? text) {
  if (text == 'auto') return QualityChoice.auto;
  final bitrateBps = int.tryParse(text ?? '');
  return bitrateBps != null && bitrateBps > 0
      ? QualityChoice.pinned(bitrateBps)
      : QualityChoice.original;
}

/// Thresholds for [AutoQuality], in milliseconds.
abstract final class Auto {
  /// After a start or a seek, filling the buffer isn't struggling.
  static const settleMs = 8000;
  static const windowMs = 60000;

  /// Stalls within [windowMs] that mean the original is too much.
  static const stalls = 2;

  /// One stall this long is enough by itself.
  static const longStallMs = 4000;

  /// How long the ladder must be comfortably over the original's rate before
  /// trying it again.
  static const recoverMs = 60000;

  /// How much over: room for the original's peaks.
  static const headroom = 1.5;

  /// The least time on the ladder after leaving the original.
  static const holdMs = 180000;
}

int _wallClock() => DateTime.now().millisecondsSinceEpoch;

/// Auto's two decisions, fed the player's events. On the original: whether
/// it's stalling enough to leave it for the ladder. On the ladder: whether the
/// connection has been comfortably above the original's bitrate for long
/// enough to go back to it. [now] is a clock in milliseconds.
class AutoQuality {
  AutoQuality([this.now = _wallClock]);

  final int Function() now;
  int _settledAt = 0;
  List<int> _stalls = const [];
  int? _stalledAt;
  int? _leftAt;
  int? _comfortableSince;

  /// The stream started or the viewer seeked: what follows is loading, not
  /// struggling.
  void settle() {
    _settledAt = now();
    _stalledAt = null;
  }

  void stalled() {
    final at = now();
    if (_stalledAt != null || at - _settledAt < Auto.settleMs) return;
    _stalledAt = at;
    _stalls = [
      for (final stall in _stalls)
        if (at - stall < Auto.windowMs) stall,
      at,
    ];
  }

  void resumed() => _stalledAt = null;

  /// On the original: whether to leave it for the ladder.
  bool struggling() {
    final at = now();
    final stalledAt = _stalledAt;
    if (stalledAt != null && at - stalledAt >= Auto.longStallMs) return true;
    return _stalls.where((stall) => at - stall < Auto.windowMs).length >=
        Auto.stalls;
  }

  /// The session moved to the ladder because the original struggled.
  void leftOriginal() {
    _leftAt = now();
    _stalls = const [];
    _comfortableSince = null;
  }

  /// On the ladder, with the player's bandwidth estimate: whether to try the
  /// original again. Only after leaving it for a while, and only once the
  /// estimate has stayed over the original's rate with headroom.
  bool recovered(int? estimateBps, int? originalBps) {
    final at = now();
    final leftAt = _leftAt;
    if (leftAt == null || originalBps == null || originalBps == 0) return false;
    if (estimateBps == null || estimateBps < originalBps * Auto.headroom) {
      _comfortableSince = null;
      return false;
    }
    final since = _comfortableSince ??= at;
    return at - leftAt >= Auto.holdMs && at - since >= Auto.recoverMs;
  }
}
