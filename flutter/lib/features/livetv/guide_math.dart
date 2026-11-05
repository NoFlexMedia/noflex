// SPDX-License-Identifier: AGPL-3.0-only
//
// The guide's arithmetic, free of widgets and generated types so it can be
// tested. Positions are minutes from the start of the window; the view decides
// how many pixels a minute is.

const guideSlot = Duration(minutes: 30);
const guideSpan = Duration(hours: 12);

/// How much guide should remain ahead of now before the window moves on.
const _guideHorizon = Duration(hours: 3);

class GuideProgram {
  const GuideProgram({
    required this.id,
    required this.channelId,
    required this.title,
    required this.start,
    required this.end,
    this.subtitle,
    this.description,
    this.category,
  });

  final String id;
  final String channelId;
  final String title;
  final String? subtitle;
  final String? description;
  final String? category;
  final DateTime start;
  final DateTime end;

  Duration get length => end.difference(start);
  bool isOn(DateTime now) => !start.isAfter(now) && end.isAfter(now);
  bool isOver(DateTime now) => !end.isAfter(now);

  /// How far through it is, from 0 to 1.
  double progress(DateTime now) {
    final total = length.inMilliseconds;
    if (total <= 0) return 0;
    return (now.difference(start).inMilliseconds / total).clamp(0.0, 1.0);
  }

  List<String> get categories => (category ?? '')
      .split(RegExp(r'\s*[,;/]\s*'))
      .where((part) => part.isNotEmpty)
      .toList();
}

/// A program as the grid places it, clipped to the window.
class GuideBlock {
  const GuideBlock(
    this.program, {
    required this.startMinutes,
    required this.lengthMinutes,
  });

  final GuideProgram program;
  final double startMinutes;
  final double lengthMinutes;
}

class GuideSlotMark {
  const GuideSlotMark(this.minutes, this.at);
  final double minutes;
  final DateTime at;

  /// Local midnight, where the grid names the new day.
  bool get startsDay {
    final local = at.toLocal();
    return local.hour == 0 && local.minute == 0;
  }
}

double _minutesBetween(DateTime from, DateTime to) =>
    to.difference(from).inMilliseconds / Duration.millisecondsPerMinute;

/// Where the window starts: the half hour before the current one, in local
/// time. Flooring on the epoch instead would put the grid's lines at :15 and
/// :45 in zones like Nepal's.
DateTime guideAnchor(DateTime now, {Duration? utcOffset}) {
  final offset = (utcOffset ?? now.timeZoneOffset).inMilliseconds;
  final slot = guideSlot.inMilliseconds;
  final local = now.millisecondsSinceEpoch + offset;
  return DateTime.fromMillisecondsSinceEpoch(
    local - local % slot - offset - slot,
  );
}

/// The window's start stays put between refreshes until this says otherwise.
bool guideWindowIsStale(DateTime anchor, DateTime now) =>
    now.isAfter(anchor.add(guideSpan - _guideHorizon));

List<GuideSlotMark> guideSlots(DateTime start, DateTime end) {
  final marks = <GuideSlotMark>[];
  for (var at = start; at.isBefore(end); at = at.add(guideSlot)) {
    marks.add(GuideSlotMark(_minutesBetween(start, at), at));
  }
  return marks;
}

/// Each channel's programs overlapping the window, in order and clipped to it.
Map<String, List<GuideBlock>> guideRows(
  Iterable<GuideProgram> programs, {
  required DateTime start,
  required DateTime end,
}) {
  final rows = <String, List<GuideBlock>>{};
  for (final program in programs) {
    if (!program.end.isAfter(start) || !program.start.isBefore(end)) continue;
    final from = program.start.isBefore(start) ? start : program.start;
    final to = program.end.isAfter(end) ? end : program.end;
    (rows[program.channelId] ??= []).add(
      GuideBlock(
        program,
        startMinutes: _minutesBetween(start, from),
        lengthMinutes: _minutesBetween(from, to),
      ),
    );
  }
  for (final row in rows.values) {
    row.sort((a, b) => a.startMinutes.compareTo(b.startMinutes));
  }
  return rows;
}

/// How far into the window now is; null when it's outside it.
double? guideNowMinutes(DateTime start, DateTime end, DateTime now) =>
    now.isBefore(start) || now.isAfter(end)
    ? null
    : _minutesBetween(start, now);

/// What's on a channel and what follows it.
({GuideProgram? current, GuideProgram? next}) nowAndNext(
  Iterable<GuideProgram> programs,
  DateTime now,
) {
  GuideProgram? current;
  GuideProgram? next;
  for (final program in programs) {
    if (program.isOn(now)) {
      current = program;
    } else if (program.start.isAfter(now) &&
        (next == null || program.start.isBefore(next.start))) {
      next = program;
    }
  }
  return (current: current, next: next);
}

/// Channels by number, then name. Unnumbered ones go last.
int compareChannels(
  String? aNumber,
  String aName,
  String? bNumber,
  String bName,
) {
  double rank(String? number) =>
      double.tryParse(number?.trim() ?? '') ?? double.infinity;
  final byNumber = rank(aNumber).compareTo(rank(bNumber));
  return byNumber != 0
      ? byNumber
      : aName.toLowerCase().compareTo(bName.toLowerCase());
}
