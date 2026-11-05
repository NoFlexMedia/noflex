// SPDX-License-Identifier: AGPL-3.0-only
//
// Numbers and words for the administration screens. Unlike utils/format.dart,
// zero is a real reading here ("0 B", "0%"), and only null means unknown.

import 'dart:math' as math;

import 'package:intl/intl.dart';

import '../../utils/format.dart';

const _units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];

/// 1536 → "2 KB"; null → "—". Binary multiples, as `df` and the OS report them.
String size(num? value, {int digits = 1}) {
  if (value == null || value.isNaN) return '—';
  if (value < 1024) return '${value.round()} B';
  var power = 0;
  var scaled = value.toDouble();
  while (scaled >= 1024 && power < _units.length - 1) {
    scaled /= 1024;
    power++;
  }
  return '${scaled.toStringAsFixed(power >= 3 ? digits : 0)} ${_units[power]}';
}

const _rates = ['b/s', 'Kb/s', 'Mb/s', 'Gb/s'];

/// Bytes per second as bits per second, the way links are rated: "12 Mb/s".
String rate(num? bytesPerSec) {
  if (bytesPerSec == null) return '—';
  final bits = bytesPerSec * 8;
  if (bits < 1000) return '${bits.round()} b/s';
  var power = 0;
  var scaled = bits.toDouble();
  while (scaled >= 1000 && power < _rates.length - 1) {
    scaled /= 1000;
    power++;
  }
  return '${scaled.toStringAsFixed(scaled < 10 ? 1 : 0)} ${_rates[power]}';
}

String percent(num? value, {int digits = 0}) =>
    value == null || value.isNaN ? '—' : '${value.toStringAsFixed(digits)}%';

/// [part] as a percentage of [whole], clamped, and 0 when either is unknown.
double ratio(num? part, num? whole) =>
    part == null || whole == null || whole == 0
    ? 0
    : (part / whole * 100).clamp(0, 100).toDouble();

/// 93784 → "1d 2h", 3720 → "1h 2m", 42 → "42s".
String uptime(int? seconds) {
  if (seconds == null) return '—';
  final d = seconds ~/ 86400;
  final h = (seconds ~/ 3600) % 24;
  final m = (seconds ~/ 60) % 60;
  if (d > 0) return '${d}d ${h}h';
  if (h > 0) return '${h}h ${m}m';
  if (m > 0) return '${m}m ${seconds % 60}s';
  return '${seconds}s';
}

String bitrate(int? bps) {
  if (bps == null || bps == 0) return '—';
  return bps >= 1000000
      ? '${(bps / 1000000).toStringAsFixed(1)} Mb/s'
      : '${(bps / 1000).round()} kb/s';
}

enum MeterTone { ok, warn, bad }

MeterTone meterTone(double pct, {double warn = 80, double bad = 92}) =>
    pct >= bad
    ? MeterTone.bad
    : pct >= warn
    ? MeterTone.warn
    : MeterTone.ok;

/// Fixed-width 24-hour time, so timestamp columns line up.
String clock(DateTime? value) =>
    value == null ? '—' : DateFormat('HH:mm:ss').format(value.toLocal());

String stamp(DateTime? value) => formatDate(value, time: true);

const _codecs = {
  'H264': 'H.264',
  'H265': 'HEVC',
  'AV1': 'AV1',
  'VP9': 'VP9',
  'VP8': 'VP8',
  'MPEG2': 'MPEG-2',
  'PRORES': 'ProRes',
  'THEORA': 'Theora',
};

String codecLabel(String? value) =>
    value == null ? '—' : _codecs[value] ?? value;

const _ranges = {
  'SDR': 'SDR',
  'HDR10': 'HDR10',
  'HDR10_PLUS': 'HDR10+',
  'HLG': 'HLG',
  'DOVI': 'Dolby Vision',
  'DOVI_WITH_SDR': 'DV · SDR',
  'DOVI_WITH_HDR10': 'DV · HDR10',
  'DOVI_WITH_HLG': 'DV · HLG',
};

String rangeLabel(String? value) =>
    value == null ? '—' : _ranges[value] ?? value;

String channelsLabel(int? count) => switch (count) {
  null || 0 => '',
  1 => 'mono',
  2 => 'stereo',
  6 => '5.1',
  8 => '7.1',
  _ => '$count ch',
};

String resolutionLabel(int? width, int? height) {
  if (width == null || height == null || width == 0 || height == 0) return '—';
  final tier = height >= 2000
      ? '4K'
      : height >= 1400
      ? '1440p'
      : height >= 1000
      ? '1080p'
      : height >= 700
      ? '720p'
      : height >= 560
      ? '576p'
      : height >= 460
      ? '480p'
      : '${height}p';
  return '$tier · $width×$height';
}

const _pixelTiers = [
  (33177600, '8K'),
  (14745600, '5K'),
  (8294400, '4K'),
  (3686400, '1440p'),
  (2073600, '1080p'),
  (921600, '720p'),
  (409920, '480p'),
  (230400, '360p'),
];

/// A pixel budget as the height it roughly holds: "~1080p".
String pixelTier(int pixels) {
  for (final (limit, name) in _pixelTiers) {
    if (pixels >= limit * 0.9) return '~$name';
  }
  return '~${math.sqrt(pixels * 9 / 16).round()}p';
}

/// 1200 → "1.2K".
String compact(num value) => NumberFormat.compact().format(value);

/// Watched time for humans: "12 min", "3.4 h", "128 h".
String watched(int? ms) {
  if (ms == null || ms == 0) return '0 min';
  final minutes = ms / 60000;
  if (minutes < 1) return '<1 min';
  if (minutes < 60) return '${minutes.round()} min';
  final hours = minutes / 60;
  return '${hours < 10 ? hours.toStringAsFixed(1) : compact(hours.round())} h';
}

/// A chart bucket's axis label, or its longer form for a tooltip.
String bucketLabel(DateTime start, String bucket, {bool long = false}) {
  final at = start.toLocal();
  String fmt(String pattern) => DateFormat(pattern).format(at);
  return switch (bucket) {
    'HOUR' => long ? fmt('EEE ha') : fmt('ha'),
    'DAY' => long ? fmt('EEEE, MMM d') : fmt('MMM d'),
    'WEEK' => long ? 'Week of ${fmt('MMM d, y')}' : fmt('MMM d'),
    _ => long ? fmt('MMMM y') : (at.month == 1 ? fmt("MMM ''yy") : fmt('MMM')),
  };
}

/// "S1 · E3 · Title" for episodes, the plain title otherwise.
String playedTitle({
  required String kind,
  required String title,
  int? parentIndex,
  int? indexNumber,
}) => kind == 'EPISODE' && indexNumber != null
    ? 'S${parentIndex ?? 0} · E$indexNumber · $title'
    : title;

/// "every 15 min", "every 6 h", "daily".
String interval(int ms) {
  final minutes = (ms / 60000).round();
  if (minutes < 1) return 'every ${math.max(1, (ms / 1000).round())} s';
  if (minutes < 60) return 'every $minutes min';
  if (minutes < 1440) return 'every ${(minutes / 60).round()} h';
  final days = (minutes / 1440).round();
  return days == 1 ? 'daily' : 'every $days days';
}

/// "just now", "3m ago", or "in 2h" for what hasn't happened yet.
String relative(DateTime? value, {DateTime? now}) {
  if (value == null) return 'never';
  final at = now ?? DateTime.now();
  if (value.isAfter(at)) {
    final delta = value.difference(at);
    if (delta.inMinutes < 1) return 'in a moment';
    if (delta.inHours < 1) return 'in ${delta.inMinutes}m';
    if (delta.inDays < 1) return 'in ${delta.inHours}h';
    return 'in ${delta.inDays}d';
  }
  return ago(value, now: at);
}

/// A comma or newline separated field's entries.
List<String> splitList(String value) => value
    .split(RegExp(r'[\n,]'))
    .map((item) => item.trim())
    .where((item) => item.isNotEmpty)
    .toList();

String plural(int count, String one, [String? many]) =>
    '$count ${count == 1 ? one : many ?? '${one}s'}';

/// Five clean steps from zero that hold [peak], so an axis reads at a glance.
/// Plays are whole; hours can be fractions.
List<double> chartTicks(double peak, {required bool whole}) {
  if (peak <= 0) {
    return [for (var n = 0; n < 5; n++) n * (whole ? 1.0 : 0.25)];
  }
  final raw = peak / 4;
  final magnitude = math
      .pow(10, (math.log(raw) / math.ln10).floor())
      .toDouble();
  var step = raw;
  for (final multiple in const [1.0, 2.0, 2.5, 5.0, 10.0]) {
    if (multiple * magnitude >= raw) {
      step = multiple * magnitude;
      break;
    }
  }
  final clean = whole ? math.max(1.0, step.ceilToDouble()) : step;
  return [for (var n = 0; n < 5; n++) n * clean];
}

/// One line of a log with the repeats folded into it.
class FoldedRow<T> {
  FoldedRow(this.first, this.oldest);
  final T first;
  T oldest;
  int count = 1;
}

/// A failing background task logs the same line every few seconds, so
/// consecutive repeats fold into one row with a count. [sameAs] says whether
/// two entries are repeats.
List<FoldedRow<T>> foldRepeats<T>(
  Iterable<T> entries,
  bool Function(T a, T b) sameAs,
) {
  final rows = <FoldedRow<T>>[];
  for (final entry in entries) {
    if (rows.isNotEmpty && sameAs(rows.last.first, entry)) {
      rows.last
        ..count += 1
        ..oldest = entry;
    } else {
      rows.add(FoldedRow(entry, entry));
    }
  }
  return rows;
}

/// When a library's next scheduled scan falls, in words.
({String label, String detail}) nextScan({
  required String schedule,
  required DateTime? last,
  required DateTime now,
}) {
  final every = switch (schedule) {
    'HOURLY' => const Duration(hours: 1),
    'DAILY' => const Duration(days: 1),
    'WEEKLY' => const Duration(days: 7),
    _ => null,
  };
  if (every == null) return (label: 'Only when you scan', detail: '');
  if (last == null) {
    return (
      label: 'Due now',
      detail: 'Never scanned — the scheduler picks it up within 5 minutes.',
    );
  }
  final at = last.add(every);
  if (!at.isAfter(now)) {
    return (label: 'Due now', detail: 'The scheduler checks every 5 minutes.');
  }
  return (label: relative(at, now: now), detail: stamp(at));
}

/// Sessions seen within this long read as active.
const sessionActiveWindow = Duration(minutes: 10);

/// One person's sessions in the full list.
class SessionGroup<T> {
  SessionGroup(this.userId, this.total);
  final String userId;
  final int total;
  final sessions = <T>[];
}

/// [sessions], newest first, as one page of per-person groups: the most
/// recently active person first, each person's newest first. A person keeps
/// their full [SessionGroup.total] even when the page cuts them short.
List<SessionGroup<T>> groupSessions<T>(
  List<T> sessions, {
  required String Function(T) userOf,
  required int page,
  required int pageSize,
}) {
  final first = <String, int>{};
  final totals = <String, int>{};
  for (final (index, session) in sessions.indexed) {
    final user = userOf(session);
    first.putIfAbsent(user, () => index);
    totals.update(user, (count) => count + 1, ifAbsent: () => 1);
  }
  final ordered = sessions.indexed.toList()
    ..sort((a, b) {
      final byPerson = first[userOf(a.$2)]!.compareTo(first[userOf(b.$2)]!);
      return byPerson != 0 ? byPerson : a.$1.compareTo(b.$1);
    });
  final groups = <SessionGroup<T>>[];
  for (final (_, session) in ordered.skip(page * pageSize).take(pageSize)) {
    final user = userOf(session);
    if (groups.isEmpty || groups.last.userId != user) {
      groups.add(SessionGroup(user, totals[user]!));
    }
    groups.last.sessions.add(session);
  }
  return groups;
}
