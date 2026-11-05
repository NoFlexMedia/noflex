// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../models/work.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/artwork.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_state.dart';
import '../../widgets/work_card.dart';
import 'account_controller.dart';
import 'account_widgets.dart';

typedef _Entry = ({String? day, HistoryRow? row});

String _dayLabel(DateTime? value, DateTime now) {
  if (value == null) return 'Undated';
  final day = value.toLocal();
  if (DateUtils.isSameDay(day, now)) return 'Today';
  if (DateUtils.isSameDay(day, DateUtils.addDaysToDate(now, -1))) {
    return 'Yesterday';
  }
  return day.year == now.year
      ? DateFormat.MMMMEEEEd().format(day)
      : DateFormat.yMMMMEEEEd().format(day);
}

List<_Entry> _grouped(List<HistoryRow> rows) {
  final now = DateTime.now();
  final entries = <_Entry>[];
  String? last;
  for (final row in rows) {
    final label = _dayLabel(row.data.lastPlayed, now);
    if (label != last) {
      entries.add((day: label, row: null));
      last = label;
    }
    entries.add((day: null, row: row));
  }
  return entries;
}

String _status(HistoryRow row) {
  final data = row.data;
  final position = data.positionMs ?? 0;
  return [
    if (data.played)
      'Finished'
    else if (position > 0)
      'Stopped at ${(position / Duration.millisecondsPerMinute).round().clamp(1, 1 << 31)} min',
    if (data.playCount > 1) 'played ${data.playCount}×',
  ].join(' · ').ifEmpty('Started');
}

extension on String {
  String ifEmpty(String fallback) => isEmpty ? fallback : this;
}

/// A sliver, so a long history builds only what's on screen.
class HistorySection extends StatelessWidget {
  const HistorySection({super.key, required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final pager = controller.history;
      final rows = pager.items.value;
      final failure = pager.error.value;
      if (rows.isEmpty) {
        if (failure != null) {
          return SliverFillRemaining(
            hasScrollBody: false,
            child: NfState.error(
              title: 'History didn’t load',
              message: failure.toString(),
              retry: pager.refresh,
            ),
          );
        }
        if (pager.loading.value || pager.hasMore.value) {
          return const SliverConstrainedCrossAxis(
            maxExtent: accountMaxWidth,
            sliver: SliverToBoxAdapter(child: NfSkeletonRows(count: 6)),
          );
        }
        return const SliverFillRemaining(
          hasScrollBody: false,
          child: NfState(
            icon: PhosphorIconsRegular.clockCounterClockwise,
            title: 'Nothing watched yet',
            message:
                'Everything you start or finish shows up here, newest first.',
          ),
        );
      }
      final entries = _grouped(rows);
      final hasMore = pager.hasMore.value;
      return SliverConstrainedCrossAxis(
        maxExtent: accountMaxWidth,
        sliver: SliverList.builder(
          itemCount: entries.length + (hasMore ? 1 : 0),
          itemBuilder: (context, index) {
            if (index == entries.length) {
              // Failing to load more leaves a button, not a loop of retries.
              if (failure == null) {
                WidgetsBinding.instance.addPostFrameCallback(
                  (_) => unawaited(pager.loadMore()),
                );
                return const Padding(
                  padding: EdgeInsets.all(16),
                  child: Center(child: CircularProgressIndicator()),
                );
              }
              return Padding(
                padding: const EdgeInsets.all(16),
                child: Center(
                  child: NfButton(
                    label: 'Load older plays',
                    onPressed: () => unawaited(pager.loadMore()),
                  ),
                ),
              );
            }
            final entry = entries[index];
            final day = entry.day;
            if (day != null) {
              return Padding(
                padding: EdgeInsets.only(top: index == 0 ? 0 : 20, bottom: 6),
                child: Text(day.toUpperCase(), style: NfType.eyebrow),
              );
            }
            return _HistoryTile(row: entry.row!);
          },
        ),
      );
    },
  );
}

class _HistoryTile extends StatelessWidget {
  const _HistoryTile({required this.row});
  final HistoryRow row;

  @override
  Widget build(BuildContext context) {
    final card = row.work;
    final lastPlayed = row.data.lastPlayed;
    final time = lastPlayed == null
        ? ''
        : DateFormat.jm().format(lastPlayed.toLocal());
    if (card == null) {
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: Row(
          children: [
            Container(
              width: 48,
              height: 72,
              decoration: BoxDecoration(
                color: NfColors.surface2,
                borderRadius: BorderRadius.circular(NfRadii.s),
              ),
              child: const Icon(
                PhosphorIconsRegular.clockCounterClockwise,
                size: 18,
                color: NfColors.text3,
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: _Lines(
                title: 'Unavailable ${row.targetKind.name.toLowerCase()}',
                titleColor: NfColors.text2,
                meta: 'Removed, or no longer shared with you',
                status: _status(row),
                done: false,
              ),
            ),
            const SizedBox(width: 8),
            Text(time, style: NfType.caption),
          ],
        ),
      );
    }
    final work = WorkSummary.fromCard(card, seriesTitle: card.seriesTitle);
    return InkWell(
      onTap: () => openWork(work),
      borderRadius: BorderRadius.circular(NfRadii.m),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: Row(
          children: [
            SizedBox(
              width: 48,
              height: 72,
              child: Artwork(work: work, radius: NfRadii.s),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: _Lines(
                title: work.heading,
                meta: [
                  work.subtitle,
                  work.episodeTitle,
                  if (!work.isEpisode) titleCase(work.kind),
                ].where((part) => part.isNotEmpty).join(' · '),
                status: _status(row),
                done: row.data.played,
              ),
            ),
            const SizedBox(width: 8),
            Text(time, style: NfType.caption),
          ],
        ),
      ),
    );
  }
}

class _Lines extends StatelessWidget {
  const _Lines({
    required this.title,
    required this.meta,
    required this.status,
    required this.done,
    this.titleColor = NfColors.text,
  });

  final String title;
  final String meta;
  final String status;
  final bool done;
  final Color titleColor;

  @override
  Widget build(BuildContext context) {
    final statusColor = done ? NfColors.success : NfColors.text3;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          title,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: nfText(14.5, weight: 600, color: titleColor),
        ),
        if (meta.isNotEmpty)
          Text(
            meta,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: NfType.caption,
          ),
        const SizedBox(height: 3),
        Row(
          children: [
            if (done) ...[
              Icon(PhosphorIconsBold.check, size: 12, color: statusColor),
              const SizedBox(width: 4),
            ],
            Expanded(
              child: Text(
                status,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: nfText(12, color: statusColor),
              ),
            ),
          ],
        ),
      ],
    );
  }
}
