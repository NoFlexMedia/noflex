// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/admin_logs.data.gql.dart';
import '../../graphql/queries/__generated__/admin_logs.req.gql.dart';
import '../../graphql/query.dart';
import '../../services/graphql.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import 'admin_format.dart';
import 'admin_widgets.dart';

typedef _Log = GAdminLogsData_recentLogs_nodes;

const _firstPage = 200;

/// A choice of level, since "All" is null to the server and null to a picker
/// means it was dismissed.
enum _Level {
  all('All levels', null),
  info('Info and up', GLogLevel.INFO),
  warn('Warnings and errors', GLogLevel.WARN),
  error('Errors only', GLogLevel.ERROR);

  const _Level(this.label, this.minLevel);
  final String label;
  final GLogLevel? minLevel;
}

const _errorFields = {'error', 'err', 'reason'};

class LogsSection extends StatefulWidget {
  const LogsSection({super.key});

  @override
  State<LogsSection> createState() => _LogsSectionState();
}

class _LogsSectionState extends State<LogsSection> {
  late final Pager<_Log> _pager;
  final _target = TextEditingController();
  final _contains = TextEditingController();
  final _level = signal(_Level.warn);
  final _follow = signal(true);
  final _open = signal<Set<String>>(const {});
  Timer? _settle;
  late final Poller _poll;
  late final void Function() _stopLevel;

  /// Following refetches the first page, which would drop older pages the admin
  /// has loaded.
  bool get _paged => _pager.items.value.length > _firstPage;

  @override
  void initState() {
    super.initState();
    _pager = Pager((after) async {
      final target = _target.text.trim();
      final contains = _contains.text.trim();
      final data = await getIt<GraphqlService>().run(
        GAdminLogsReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NoCache
            ..vars.minLevel = _level.peek().minLevel
            ..vars.target = target.isEmpty ? null : target
            ..vars.contains = contains.isEmpty ? null : contains
            ..vars.after = after,
        ),
      );
      final info = data.recentLogs.pageInfo;
      return (
        data.recentLogs.nodes.toList(),
        info.hasNextPage ? info.endCursor : null,
      );
    });
    _stopLevel = effect(() {
      _level.value;
      untracked(() => unawaited(_pager.refresh()));
    });
    _poll = Poller(() {
      if (_follow.value && !_paged) unawaited(_pager.refresh());
    }, const Duration(seconds: 5));
  }

  @override
  void dispose() {
    _stopLevel();
    _poll.cancel();
    _settle?.cancel();
    _target.dispose();
    _contains.dispose();
    super.dispose();
  }

  /// Text filters settle before they refetch, so typing doesn't spam the server.
  void _filterChanged() {
    _settle?.cancel();
    _settle = Timer(
      const Duration(milliseconds: 300),
      () => unawaited(_pager.refresh()),
    );
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final level = _level.value;
      final rows = foldRepeats<_Log>(
        _pager.items.value,
        (a, b) =>
            a.level == b.level &&
            a.target == b.target &&
            a.message == b.message,
      );
      return AdminStack(
        children: [
          AdminColumns(
            minWidth: 210,
            gap: 10,
            children: [
              AdminSelect<_Level>(
                placeholder: 'Minimum level',
                options: [
                  for (final item in _Level.values) NfOption(item, item.label),
                ],
                value: level,
                onChanged: (value) => _level.value = value,
              ),
              NfField(
                controller: _target,
                hint: 'Module, e.g. noflex_scanner',
                autocorrect: false,
                onChanged: (_) => _filterChanged(),
              ),
              NfField(
                controller: _contains,
                hint: 'Search messages and fields',
                autocorrect: false,
                onChanged: (_) => _filterChanged(),
              ),
            ],
          ),
          Row(
            children: [
              SizedBox(
                width: 130,
                child: Tooltip(
                  message: _paged
                      ? 'Paused while older entries are loaded'
                      : 'Refresh the newest entries every 5 s',
                  child: AdminCheck(
                    label: 'Follow',
                    value: _follow.value,
                    onChanged: _paged ? null : (value) => _follow.value = value,
                  ),
                ),
              ),
              NfIconButton(
                icon: PhosphorIconsRegular.arrowsClockwise,
                label: 'Refresh',
                size: 34,
                iconSize: 16,
                onPressed: () => unawaited(_pager.refresh()),
              ),
            ],
          ),
          Text(
            'The server keeps the last 2,000 events at info level and above, and its last 1,000 warnings and errors, in memory. They reset when it restarts.',
            style: NfType.caption,
          ),
          if (_pager.waiting.value)
            const Padding(
              padding: EdgeInsets.all(40),
              child: Center(child: CircularProgressIndicator()),
            )
          else if (rows.isEmpty && _pager.error.value != null)
            NfState.error(
              title: 'Couldn’t read the log',
              message: _pager.error.value.toString(),
              retry: () => unawaited(_pager.refresh()),
            )
          else if (rows.isEmpty)
            NfPanel(
              child: NfState(
                icon: PhosphorIconsRegular.checkCircle,
                title: 'Nothing logged',
                message: level == _Level.all || level == _Level.info
                    ? 'No events match these filters.'
                    : 'No warnings or errors since the server started.',
              ),
            )
          else
            NfPanel(
              padding: const EdgeInsets.all(6),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final (index, row) in rows.indexed)
                    Container(
                      decoration: BoxDecoration(
                        border: index == 0
                            ? null
                            : const Border(
                                top: BorderSide(color: NfColors.line),
                              ),
                      ),
                      child: _LogRow(
                        row: row,
                        open: _open.value.contains(row.first.id),
                        onToggle: () {
                          final id = row.first.id;
                          _open.value = _open.value.contains(id)
                              ? ({..._open.value}..remove(id))
                              : {..._open.value, id};
                        },
                      ),
                    ),
                ],
              ),
            ),
          if (_pager.hasMore.value && rows.isNotEmpty)
            Center(
              child: NfButton(
                label: 'Load older',
                size: NfButtonSize.small,
                busy: _pager.loading.value,
                onPressed: () => unawaited(_pager.loadMore()),
              ),
            ),
        ],
      );
    },
  );
}

class _LogRow extends StatelessWidget {
  const _LogRow({
    required this.row,
    required this.open,
    required this.onToggle,
  });
  final FoldedRow<_Log> row;
  final bool open;
  final VoidCallback onToggle;

  @override
  Widget build(BuildContext context) {
    final node = row.first;
    final expandable = node.fields.isNotEmpty || node.spans.isNotEmpty;
    final levelColor = switch (node.level) {
      GLogLevel.WARN => NfColors.warning,
      GLogLevel.ERROR => NfColors.danger,
      _ => NfColors.text3,
    };
    String? errorText;
    for (final field in node.fields) {
      if (_errorFields.contains(field.name)) {
        errorText = field.value;
        break;
      }
    }
    final message = node.message.isNotEmpty
        ? node.message
        : node.fields.map((f) => '${f.name}=${f.value}').join(' ');
    final target = node.target
        .replaceFirst('noflex_', '')
        .replaceAll('::', '·');
    return InkWell(
      borderRadius: BorderRadius.circular(NfRadii.s),
      onTap: expandable ? onToggle : null,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Tooltip(
                  message: stamp(node.at),
                  child: Text(
                    clock(node.at),
                    style: nfText(
                      12,
                      color: NfColors.text3,
                      features: NfType.tabular,
                    ),
                  ),
                ),
                const SizedBox(width: 10),
                Text(
                  node.level.name,
                  style: nfText(
                    10.5,
                    weight: 700,
                    color: levelColor,
                    letterSpacing: 0.6,
                  ),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Tooltip(
                    message: node.target,
                    child: Text(
                      target,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: NfType.mono.copyWith(
                        fontSize: 11.5,
                        color: NfColors.text3,
                      ),
                    ),
                  ),
                ),
                if (expandable)
                  Icon(
                    open
                        ? PhosphorIconsRegular.caretUp
                        : PhosphorIconsRegular.caretDown,
                    size: 14,
                    color: NfColors.text3,
                  ),
              ],
            ),
            const SizedBox(height: 3),
            Text.rich(
              TextSpan(
                children: [
                  if (row.count > 1)
                    WidgetSpan(
                      alignment: PlaceholderAlignment.middle,
                      child: Tooltip(
                        message:
                            'Repeated ${row.count} times since ${stamp(row.oldest.at)}',
                        child: Container(
                          margin: const EdgeInsets.only(right: 6),
                          padding: const EdgeInsets.symmetric(horizontal: 6),
                          decoration: BoxDecoration(
                            color: NfColors.surface3,
                            borderRadius: BorderRadius.circular(5),
                          ),
                          child: Text(
                            '×${row.count}',
                            style: nfText(
                              11.5,
                              weight: 700,
                              color: NfColors.text2,
                            ),
                          ),
                        ),
                      ),
                    ),
                  TextSpan(
                    text: message,
                    style: nfText(
                      13.5,
                      color: node.level == GLogLevel.ERROR
                          ? NfColors.text
                          : NfColors.text2,
                    ),
                  ),
                ],
              ),
            ),
            if (errorText != null && node.message.isNotEmpty) ...[
              const SizedBox(height: 2),
              Text(
                errorText,
                maxLines: open ? null : 1,
                overflow: open ? null : TextOverflow.ellipsis,
                style: NfType.mono.copyWith(
                  fontSize: 11.5,
                  color: node.level == GLogLevel.ERROR
                      ? NfColors.danger
                      : NfColors.text3,
                ),
              ),
            ],
            if (open)
              Container(
                margin: const EdgeInsets.only(top: 8),
                padding: const EdgeInsets.all(10),
                decoration: BoxDecoration(
                  color: const Color(0x38000000),
                  borderRadius: BorderRadius.circular(NfRadii.s),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    if (node.spans.isNotEmpty) ...[
                      SelectableText(
                        node.spans.join(' › '),
                        style: NfType.mono.copyWith(
                          fontSize: 11.5,
                          color: NfColors.focus,
                        ),
                      ),
                      const SizedBox(height: 8),
                    ],
                    for (final field in node.fields)
                      Padding(
                        padding: const EdgeInsets.only(bottom: 4),
                        child: SelectableText.rich(
                          TextSpan(
                            style: NfType.mono.copyWith(fontSize: 11.5),
                            children: [
                              TextSpan(
                                text: '${field.name}  ',
                                style: const TextStyle(color: NfColors.text3),
                              ),
                              TextSpan(text: field.value),
                            ],
                          ),
                        ),
                      ),
                  ],
                ),
              ),
          ],
        ),
      ),
    );
  }
}
