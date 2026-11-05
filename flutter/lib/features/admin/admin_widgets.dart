// SPDX-License-Identifier: AGPL-3.0-only
//
// What the administration screens are built from: status pills, meters, charts,
// and tables that turn into lists when there's no room for columns.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../layout/window.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import 'admin_format.dart';

enum StateTone { none, ok, info, accent, warn, bad }

({Color fg, Color bg}) toneColors(StateTone tone) => switch (tone) {
  StateTone.none => (fg: NfColors.text2, bg: NfColors.surface2),
  StateTone.ok => (fg: NfColors.success, bg: NfColors.successSoft),
  StateTone.info => (fg: NfColors.focus, bg: const Color(0x1FA5DCFF)),
  StateTone.accent => (fg: NfColors.accent, bg: NfColors.accentSoft),
  StateTone.warn => (fg: NfColors.warning, bg: const Color(0x1FFFC164)),
  StateTone.bad => (fg: NfColors.danger, bg: NfColors.dangerSoft),
};

/// A small labelled status, the web's `.state`.
class StatePill extends StatelessWidget {
  const StatePill(
    this.label, {
    super.key,
    this.tone = StateTone.none,
    this.icon,
    this.busy = false,
  });

  final String label;
  final StateTone tone;
  final IconData? icon;
  final bool busy;

  @override
  Widget build(BuildContext context) {
    final colors = toneColors(tone);
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 4),
      decoration: BoxDecoration(
        color: colors.bg,
        borderRadius: BorderRadius.circular(NfRadii.pill),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (busy) ...[
            SizedBox.square(
              dimension: 10,
              child: CircularProgressIndicator(
                strokeWidth: 1.6,
                color: colors.fg,
              ),
            ),
            const SizedBox(width: 6),
          ] else if (icon != null) ...[
            Icon(icon, size: 12, color: colors.fg),
            const SizedBox(width: 5),
          ],
          Flexible(
            child: Text(
              label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: nfText(11.5, weight: 600, color: colors.fg),
            ),
          ),
        ],
      ),
    );
  }
}

/// An outlined tag: a hook, a provider, a capability.
class TagChip extends StatelessWidget {
  const TagChip(this.label, {super.key, this.mono = false});
  final String label;
  final bool mono;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
    decoration: BoxDecoration(
      color: NfColors.surface,
      borderRadius: BorderRadius.circular(NfRadii.xs),
      border: Border.all(color: NfColors.line),
    ),
    child: Text(
      label,
      style: mono
          ? NfType.mono.copyWith(fontSize: 11.5)
          : nfText(12, weight: 500, color: NfColors.text2),
    ),
  );
}

/// Sections stacked with the same space between them.
class AdminStack extends StatelessWidget {
  const AdminStack({super.key, required this.children, this.gap = 14});
  final List<Widget> children;
  final double gap;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      for (final (index, child) in children.indexed) ...[
        if (index > 0) SizedBox(height: gap),
        child,
      ],
    ],
  );
}

/// As many equal columns as fit at [minWidth], however wide the pane is. Pane
/// width rather than window width, since a section may sit beside a list.
class AdminColumns extends StatelessWidget {
  const AdminColumns({
    super.key,
    required this.children,
    this.minWidth = 420,
    this.gap = 14,
  });

  final List<Widget> children;
  final double minWidth;
  final double gap;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final width = constraints.maxWidth;
      final count = math.max(1, ((width + gap) / (minWidth + gap)).floor());
      final cell = (width - gap * (count - 1)) / count;
      return Wrap(
        spacing: gap,
        runSpacing: gap,
        children: [
          for (final child in children) SizedBox(width: cell, child: child),
        ],
      );
    },
  );
}

class Fact {
  const Fact(
    this.label,
    this.value, {
    this.mono = false,
    this.full = false,
    this.color,
    this.child,
  });

  final String label;
  final String value;
  final bool mono;

  /// Takes its own line: paths and other long values.
  final bool full;
  final Color? color;

  /// Shown in place of [value].
  final Widget? child;
}

/// Labelled values in a grid, the web's `dl.kv`. Monospace ones can be selected,
/// since they're paths and names people copy elsewhere.
class Facts extends StatelessWidget {
  const Facts(this.facts, {super.key, this.minWidth = 130});
  final List<Fact> facts;
  final double minWidth;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      const gap = 18.0;
      final width = constraints.maxWidth;
      final count = math.max(1, ((width + gap) / (minWidth + gap)).floor());
      final cell = (width - gap * (count - 1)) / count;
      return Wrap(
        spacing: gap,
        runSpacing: 12,
        children: [
          for (final fact in facts)
            SizedBox(
              width: fact.full ? width : cell,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(fact.label.toUpperCase(), style: NfType.eyebrow),
                  const SizedBox(height: 4),
                  fact.child ??
                      (fact.mono
                          ? SelectableText(fact.value, style: NfType.mono)
                          : Text(
                              fact.value,
                              style: nfText(
                                13.5,
                                weight: 550,
                                color: fact.color ?? NfColors.text,
                              ),
                            )),
                ],
              ),
            ),
        ],
      );
    },
  );
}

/// A utilisation bar. Its color follows [warn] and [bad].
class Meter extends StatelessWidget {
  const Meter({
    super.key,
    required this.pct,
    this.warn = 80,
    this.bad = 92,
    this.thin = false,
    this.label,
  });

  final double pct;
  final double warn;
  final double bad;
  final bool thin;
  final String? label;

  @override
  Widget build(BuildContext context) {
    final clamped = pct.clamp(0.0, 100.0);
    final color = switch (meterTone(clamped, warn: warn, bad: bad)) {
      MeterTone.ok => NfColors.accent,
      MeterTone.warn => NfColors.warning,
      MeterTone.bad => NfColors.danger,
    };
    return Semantics(
      label: label,
      value: percent(clamped),
      child: NfProgressBar(
        value: clamped / 100,
        height: thin ? 4 : 6,
        color: color,
      ),
    );
  }
}

/// An inline sparkline. [values] run oldest first; [max] pins the top, or it
/// scales to the window's peak. [alt] is a second series, dashed, on one scale.
class Sparkline extends StatelessWidget {
  const Sparkline({
    super.key,
    required this.values,
    this.alt,
    this.max,
    this.color = NfColors.accent,
    this.altColor = NfColors.focus,
    this.height = 36,
    this.label,
  });

  final List<double> values;
  final List<double>? alt;
  final double? max;
  final Color color;
  final Color altColor;
  final double height;
  final String? label;

  @override
  Widget build(BuildContext context) => Semantics(
    label: label,
    image: true,
    child: SizedBox(
      height: height,
      width: double.infinity,
      child: CustomPaint(
        painter: _SparkPainter(values, alt, max, color, altColor),
      ),
    ),
  );
}

class _SparkPainter extends CustomPainter {
  const _SparkPainter(
    this.values,
    this.alt,
    this.max,
    this.color,
    this.altColor,
  );

  final List<double> values;
  final List<double>? alt;
  final double? max;
  final Color color;
  final Color altColor;

  Path _line(List<double> series, Size size, double top) {
    final path = Path();
    final step = size.width / (series.length - 1);
    for (final (index, value) in series.indexed) {
      final y =
          size.height - (math.min(value, top) / top) * (size.height - 2) - 1;
      if (index == 0) {
        path.moveTo(0, y);
      } else {
        path.lineTo(index * step, y);
      }
    }
    return path;
  }

  @override
  void paint(Canvas canvas, Size size) {
    if (values.length < 2) return;
    var peak = 1e-9;
    for (final value in [...values, ...?alt]) {
      if (value > peak) peak = value;
    }
    final top = max ?? peak * 1.15;
    final line = _line(values, size, top);
    final area = Path.from(line)
      ..lineTo(size.width, size.height)
      ..lineTo(0, size.height)
      ..close();
    canvas.drawPath(
      area,
      Paint()
        ..shader = LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [color.withValues(alpha: 0.28), color.withValues(alpha: 0)],
        ).createShader(Offset.zero & size),
    );
    canvas.drawPath(
      line,
      Paint()
        ..color = color
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.6
        ..strokeJoin = StrokeJoin.round,
    );
    final second = alt;
    if (second != null && second.length >= 2) {
      final paint = Paint()
        ..color = altColor.withValues(alpha: 0.6)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.3;
      for (final metric in _line(second, size, top).computeMetrics()) {
        for (var at = 0.0; at < metric.length; at += 6) {
          canvas.drawPath(metric.extractPath(at, at + 3), paint);
        }
      }
    }
  }

  @override
  bool shouldRepaint(_SparkPainter old) =>
      old.values != values || old.alt != alt || old.max != max;
}

/// A headline number that opens a section.
class StatTile extends StatelessWidget {
  const StatTile({
    super.key,
    required this.icon,
    required this.label,
    required this.value,
    this.small,
    this.sub,
    this.spark,
    this.tone = StateTone.none,
    this.onTap,
  });

  final IconData icon;
  final String label;
  final String value;
  final String? small;
  final String? sub;
  final Widget? spark;
  final StateTone tone;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final color = tone == StateTone.none ? NfColors.text : toneColors(tone).fg;
    return NfPanel(
      padding: const EdgeInsets.all(14),
      onTap: onTap,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Icon(icon, size: 13, color: NfColors.text3),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: nfText(12, weight: 600, color: NfColors.text3),
                ),
              ),
            ],
          ),
          const SizedBox(height: 6),
          Text.rich(
            TextSpan(
              children: [
                TextSpan(
                  text: value,
                  style: nfText(
                    20,
                    weight: 700,
                    color: color,
                    letterSpacing: -0.5,
                    features: NfType.tabular,
                  ),
                ),
                if (small != null)
                  TextSpan(text: '  $small', style: NfType.caption),
              ],
            ),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
          if (sub != null) ...[
            const SizedBox(height: 2),
            Text(
              sub!,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: NfType.caption,
            ),
          ],
          if (spark != null) ...[const SizedBox(height: 8), spark!],
        ],
      ),
    );
  }
}

class AdminCol {
  const AdminCol(this.label, {this.numeric = false, this.flex = false});
  final String label;
  final bool numeric;

  /// Takes the room the other columns leave: a path, a description.
  final bool flex;
}

/// Columns when they fit, and otherwise one small card per row, with the first
/// cell as its title and the rest labelled.
class AdminTable extends StatelessWidget {
  const AdminTable({
    super.key,
    required this.columns,
    required this.rows,
    this.minWidth = 620,
    this.empty,
  });

  final List<AdminCol> columns;
  final List<List<Widget>> rows;
  final double minWidth;
  final String? empty;

  @override
  Widget build(BuildContext context) {
    if (rows.isEmpty) return Text(empty ?? '', style: NfType.caption);
    return LayoutBuilder(
      builder: (context, constraints) =>
          constraints.maxWidth >= minWidth ? _table() : _cards(),
    );
  }

  Widget _table() => Table(
    defaultVerticalAlignment: TableCellVerticalAlignment.middle,
    columnWidths: {
      for (final (index, column) in columns.indexed)
        index: column.flex
            ? const FlexColumnWidth()
            : const IntrinsicColumnWidth(),
    },
    children: [
      TableRow(
        children: [
          for (final column in columns)
            Padding(
              padding: const EdgeInsets.only(right: 16, bottom: 8),
              child: Text(
                column.label.toUpperCase(),
                textAlign: column.numeric ? TextAlign.right : TextAlign.left,
                style: NfType.eyebrow,
              ),
            ),
        ],
      ),
      for (final row in rows)
        TableRow(
          decoration: const BoxDecoration(
            border: Border(top: BorderSide(color: NfColors.line)),
          ),
          children: [
            for (final (index, cell) in row.indexed)
              Padding(
                padding: const EdgeInsets.only(right: 16, top: 8, bottom: 8),
                child: Align(
                  alignment: columns[index].numeric
                      ? Alignment.centerRight
                      : Alignment.centerLeft,
                  widthFactor: 1,
                  child: cell,
                ),
              ),
          ],
        ),
    ],
  );

  Widget _cards() => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      for (final (rowIndex, row) in rows.indexed)
        Container(
          padding: const EdgeInsets.symmetric(vertical: 10),
          decoration: BoxDecoration(
            border: rowIndex == 0
                ? null
                : const Border(top: BorderSide(color: NfColors.line)),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              row.first,
              const SizedBox(height: 6),
              Wrap(
                spacing: 16,
                runSpacing: 8,
                children: [
                  for (final (index, cell) in row.indexed.skip(1))
                    if (columns[index].label.isEmpty)
                      cell
                    else
                      Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Text(
                            columns[index].label.toUpperCase(),
                            style: nfText(
                              10,
                              weight: 600,
                              color: NfColors.text3,
                              letterSpacing: 1.2,
                            ),
                          ),
                          const SizedBox(height: 2),
                          cell,
                        ],
                      ),
                ],
              ),
            ],
          ),
        ),
    ],
  );
}

/// Cell text for a table.
Widget cellText(String value, {bool strong = false, Color? color}) => Text(
  value,
  style: nfText(
    13,
    weight: strong ? 600 : 400,
    color: color ?? (strong ? NfColors.text : NfColors.text2),
    features: NfType.tabular,
  ),
);

Widget cellMono(String value) =>
    SelectableText(value, style: NfType.mono.copyWith(fontSize: 12));

/// A problem stated in a panel.
class Notice extends StatelessWidget {
  const Notice(this.message, {super.key, this.warn = false});
  final String message;
  final bool warn;

  @override
  Widget build(BuildContext context) {
    final color = warn ? NfColors.warning : NfColors.danger;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.only(top: 2),
          child: Icon(
            PhosphorIconsRegular.warningCircle,
            size: 14,
            color: color,
          ),
        ),
        const SizedBox(width: 8),
        Expanded(
          child: Text(message, style: nfText(13, color: color)),
        ),
      ],
    );
  }
}

/// A form control that opens a list of choices. A choice can't be null, since
/// that's what the list answers when it's dismissed.
class AdminSelect<T extends Object> extends StatelessWidget {
  const AdminSelect({
    super.key,
    required this.options,
    required this.value,
    required this.onChanged,
    this.label,
    this.placeholder = 'Choose…',
  });

  final String? label;
  final List<NfOption<T>> options;
  final T? value;
  final ValueChanged<T>? onChanged;
  final String placeholder;

  @override
  Widget build(BuildContext context) {
    String? current;
    for (final option in options) {
      if (option.value == value) current = option.label;
    }
    final title = label ?? placeholder;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (label != null) ...[
          Text(label!, style: nfText(13, weight: 600, color: NfColors.text2)),
          const SizedBox(height: 6),
        ],
        Semantics(
          button: true,
          label: title,
          value: current,
          excludeSemantics: true,
          child: Material(
            color: NfColors.surface,
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(NfRadii.m),
              side: const BorderSide(color: NfColors.lineStrong),
            ),
            clipBehavior: Clip.antiAlias,
            child: InkWell(
              onTap: onChanged == null
                  ? null
                  : () async {
                      final picked = await pickOption<T>(
                        title: title,
                        options: options,
                        selected: value,
                      );
                      if (picked != null) onChanged!(picked);
                    },
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 14,
                  vertical: 12,
                ),
                child: Row(
                  children: [
                    Expanded(
                      child: Text(
                        current ?? placeholder,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: nfText(
                          14.5,
                          color: current == null
                              ? NfColors.text3
                              : NfColors.text,
                        ),
                      ),
                    ),
                    const SizedBox(width: 8),
                    const Icon(
                      PhosphorIconsRegular.caretDown,
                      size: 14,
                      color: NfColors.text2,
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}

/// Mutually exclusive chips, the web's segmented control.
class AdminSegments<T> extends StatelessWidget {
  const AdminSegments({
    super.key,
    required this.items,
    required this.value,
    required this.onChanged,
  });

  final List<(T, String)> items;
  final T value;
  final ValueChanged<T> onChanged;

  @override
  Widget build(BuildContext context) => Wrap(
    spacing: 6,
    runSpacing: 6,
    children: [
      for (final (item, label) in items)
        NfChoiceChip(
          label: label,
          selected: item == value,
          onTap: () => onChanged(item),
        ),
    ],
  );
}

/// The frame of a form or list opened over a section: a sheet on a phone, a
/// dialog with room. The title and buttons stay put while the body scrolls.
class AdminSheet extends StatelessWidget {
  const AdminSheet({
    super.key,
    required this.title,
    required this.child,
    this.description,
    this.footer = const [],
  });

  final String title;
  final String? description;
  final Widget child;
  final List<Widget> footer;

  @override
  Widget build(BuildContext context) => Padding(
    // A dialog already lifts itself above the keyboard; a sheet has to ask.
    padding: EdgeInsets.only(
      bottom: context.windowClass.isCompact
          ? MediaQuery.viewInsetsOf(context).bottom
          : 0,
    ),
    child: SafeArea(
      top: false,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(22, 20, 22, 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title, style: NfType.sectionTitle),
                if (description != null) ...[
                  const SizedBox(height: 4),
                  Text(description!, style: NfType.small),
                ],
              ],
            ),
          ),
          Flexible(
            child: SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(22, 4, 22, 12),
              child: child,
            ),
          ),
          if (footer.isNotEmpty)
            Padding(
              padding: const EdgeInsets.fromLTRB(22, 8, 22, 18),
              child: Wrap(
                alignment: WrapAlignment.end,
                spacing: 10,
                runSpacing: 8,
                children: footer,
              ),
            ),
        ],
      ),
    ),
  );
}

/// A heading inside a form.
class FormHeading extends StatelessWidget {
  const FormHeading(this.label, {super.key});
  final String label;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 10),
    child: Text(label.toUpperCase(), style: NfType.eyebrow),
  );
}

/// What a field is for, under the field.
class FieldNote extends StatelessWidget {
  const FieldNote(this.text, {super.key});
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 5),
    child: Text(text, style: NfType.caption),
  );
}

/// A checkbox row.
class AdminCheck extends StatelessWidget {
  const AdminCheck({
    super.key,
    required this.label,
    required this.value,
    required this.onChanged,
    this.detail,
  });

  final String label;
  final String? detail;
  final bool value;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) => InkWell(
    borderRadius: BorderRadius.circular(NfRadii.s),
    onTap: onChanged == null ? null : () => onChanged!(!value),
    child: Row(
      children: [
        Checkbox(
          value: value,
          onChanged: onChanged == null ? null : (next) => onChanged!(next!),
        ),
        const SizedBox(width: 4),
        Expanded(child: Text(label, style: nfText(14))),
        if (detail != null) Text(detail!, style: NfType.caption),
      ],
    ),
  );
}

/// Runs [tick] on a schedule for as long as a section is on screen. Sections
/// own one and cancel it on dispose; the web's `poll` does the same.
class Poller {
  Poller(void Function() tick, Duration every)
    : _timer = Timer.periodic(every, (_) => tick());

  final Timer _timer;

  void cancel() => _timer.cancel();
}

/// A column chart of [points] over time, one series. Hovering or tapping a bar
/// names it.
class PlayChart extends StatefulWidget {
  const PlayChart({
    super.key,
    required this.points,
    required this.bucket,
    required this.showHours,
    this.label,
  });

  final List<({DateTime start, int plays, int watchedMs})> points;
  final String bucket;
  final bool showHours;
  final String? label;

  @override
  State<PlayChart> createState() => _PlayChartState();
}

class _PlayChartState extends State<PlayChart> {
  static const _axis = 40.0;
  static const _labels = 22.0;
  final _hovered = signal<int?>(null);

  double _value(({DateTime start, int plays, int watchedMs}) point) =>
      widget.showHours ? point.watchedMs / 3600000 : point.plays.toDouble();

  int _indexAt(double x, double width) {
    final plot = width - _axis;
    return (((x - _axis) / plot) * widget.points.length).floor().clamp(
      0,
      widget.points.length - 1,
    );
  }

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final width = constraints.maxWidth;
      final height = width < 480 ? 180.0 : 220.0;
      final values = widget.points.map(_value).toList();
      final ticks = chartTicks(
        values.fold(0.0, math.max),
        whole: !widget.showHours,
      );
      return Semantics(
        label: widget.label,
        image: true,
        child: MouseRegion(
          onHover: (event) =>
              _hovered.value = _indexAt(event.localPosition.dx, width),
          onExit: (_) => _hovered.value = null,
          child: GestureDetector(
            onTapDown: (details) {
              final index = _indexAt(details.localPosition.dx, width);
              _hovered.value = _hovered.value == index ? null : index;
            },
            child: SizedBox(
              width: width,
              height: height + _labels,
              child: SignalBuilder(
                builder: (context) {
                  final hovered = _hovered.value;
                  return Stack(
                    clipBehavior: Clip.none,
                    children: [
                      Positioned.fill(
                        child: CustomPaint(
                          painter: _ChartPainter(
                            values: values,
                            ticks: ticks,
                            hovered: hovered,
                            axis: _axis,
                            labelsHeight: _labels,
                            xLabel: (index) => bucketLabel(
                              widget.points[index].start,
                              widget.bucket,
                            ),
                          ),
                        ),
                      ),
                      if (hovered != null && hovered < widget.points.length)
                        _tip(hovered, width, height, values, ticks.last),
                    ],
                  );
                },
              ),
            ),
          ),
        ),
      );
    },
  );

  Widget _tip(
    int index,
    double width,
    double height,
    List<double> values,
    double top,
  ) {
    final point = widget.points[index];
    final slot = (width - _axis) / widget.points.length;
    final x = _axis + slot * (index + 0.5);
    final fromBottom = math.min(
      (top == 0 ? 0 : values[index] / top) * height + 10 + _labels,
      height - 30,
    );
    final style = nfText(12, color: const Color(0xFF0B0C0F));
    return Positioned(
      bottom: fromBottom,
      left: x < width / 2 ? math.max(0, x - 40) : null,
      right: x < width / 2 ? null : math.max(0, width - x - 40),
      child: IgnorePointer(
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7),
          decoration: BoxDecoration(
            color: NfColors.text,
            borderRadius: BorderRadius.circular(NfRadii.s),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                bucketLabel(point.start, widget.bucket, long: true),
                style: nfText(12, weight: 700, color: const Color(0xFF0B0C0F)),
              ),
              Text(
                '${plural(point.plays, 'play')} · ${watched(point.watchedMs)}',
                style: style,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _ChartPainter extends CustomPainter {
  const _ChartPainter({
    required this.values,
    required this.ticks,
    required this.hovered,
    required this.axis,
    required this.labelsHeight,
    required this.xLabel,
  });

  final List<double> values;
  final List<double> ticks;
  final int? hovered;
  final double axis;
  final double labelsHeight;
  final String Function(int) xLabel;

  static String _tickText(double value) {
    if (value == 0) return '0';
    if (value < 10 && value != value.roundToDouble()) {
      return value.toStringAsFixed(value < 1 ? 2 : 1);
    }
    return compact(value.round());
  }

  void _text(Canvas canvas, String text, Offset at, {bool right = false}) {
    final painter = TextPainter(
      text: TextSpan(
        text: text,
        style: nfText(11, color: NfColors.text3),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    painter.paint(
      canvas,
      right
          ? at - Offset(painter.width, painter.height / 2)
          : at - Offset(painter.width / 2, 0),
    );
    painter.dispose();
  }

  @override
  void paint(Canvas canvas, Size size) {
    final plotHeight = size.height - labelsHeight;
    final plotWidth = size.width - axis;
    final top = ticks.last == 0 ? 1.0 : ticks.last;
    for (final (index, tick) in ticks.indexed) {
      final y = plotHeight - (tick / top) * plotHeight;
      canvas.drawLine(
        Offset(axis, y),
        Offset(size.width, y),
        Paint()..color = index == 0 ? NfColors.lineStrong : NfColors.line,
      );
      _text(canvas, _tickText(tick), Offset(axis - 8, y), right: true);
    }
    if (values.isEmpty) return;
    final slot = plotWidth / values.length;
    final barWidth = math.min(24.0, math.max(1.0, slot - 2));
    for (final (index, value) in values.indexed) {
      final x = axis + slot * index + (slot - barWidth) / 2;
      final barHeight = math.max(2.0, (value / top) * plotHeight);
      canvas.drawRRect(
        RRect.fromRectAndCorners(
          Rect.fromLTWH(x, plotHeight - barHeight, barWidth, barHeight),
          topLeft: const Radius.circular(3),
          topRight: const Radius.circular(3),
        ),
        Paint()
          ..color = value == 0
              ? NfColors.lineStrong
              : NfColors.accent.withValues(alpha: index == hovered ? 1 : 0.85),
      );
    }
    // Thinned so they never collide.
    final across = math.max(2, (plotWidth / 64).floor());
    final every = (values.length / across).ceil();
    for (var index = 0; index < values.length; index += every) {
      _text(
        canvas,
        xLabel(index),
        Offset(axis + slot * (index + 0.5), plotHeight + 6),
      );
    }
  }

  @override
  bool shouldRepaint(_ChartPainter old) =>
      old.values != values || old.hovered != hovered || old.ticks != ticks;
}
