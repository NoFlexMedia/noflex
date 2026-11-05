// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/admin_indexing.data.gql.dart';
import '../../graphql/queries/__generated__/admin_indexing.req.gql.dart';
import '../../graphql/queries/__generated__/admin_indexing.var.gql.dart';
import '../../graphql/queries/__generated__/clear_embedding_endpoint.req.gql.dart';
import '../../graphql/queries/__generated__/embedding_endpoint_settings.data.gql.dart';
import '../../graphql/queries/__generated__/embedding_endpoint_settings.req.gql.dart';
import '../../graphql/queries/__generated__/embedding_endpoint_settings.var.gql.dart';
import '../../graphql/queries/__generated__/save_embedding_endpoint.req.gql.dart';
import '../../graphql/queries/__generated__/trigger_library_scan.req.gql.dart';
import '../../graphql/query.dart';
import '../../services/graphql.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_state.dart';
import '../account/account_widgets.dart';
import 'admin_activity.dart';
import 'admin_controller.dart';
import 'admin_format.dart';
import 'admin_libraries.dart';
import 'admin_widgets.dart';

typedef _Entry = GAdminIndexingData_scannerStatus;

DateTime? _lastAt(_Entry entry) =>
    entry.lastScan?.finishedAt ??
    entry.lastScan?.startedAt ??
    DateTime.tryParse(entry.scan.finishedAt ?? '');

bool _running(_Entry entry) =>
    entry.activeScan != null || entry.scan.state == GScanState.RUNNING;

class IndexingSection extends StatefulWidget {
  const IndexingSection({super.key, required this.controller});
  final AdminController controller;

  @override
  State<IndexingSection> createState() => _IndexingSectionState();
}

class _IndexingSectionState extends State<IndexingSection> {
  late final Query<GAdminIndexingData, GAdminIndexingVars> _indexing;
  late final Query<
    GEmbeddingEndpointSettingsData,
    GEmbeddingEndpointSettingsVars
  >
  _endpoint;
  late final void Function() _stopCadence;
  Poller? _poll;
  bool? _pollingBusy;
  final _now = signal(DateTime.now());
  final _starting = signal<String?>(null);
  final _expanded = signal<Set<String>>(const {});
  final _editing = signal(false);
  final _saving = signal(false);

  final _url = TextEditingController();
  final _apiKey = TextEditingController();
  final _model = TextEditingController();
  final _dimensions = TextEditingController();
  final _queryPrefix = TextEditingController();
  final _documentPrefix = TextEditingController();

  @override
  void initState() {
    super.initState();
    final client = getIt<GraphqlService>().client;
    _indexing = Query(
      client,
      GAdminIndexingReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    _endpoint = Query(
      client,
      GEmbeddingEndpointSettingsReq(
        (b) => b..fetchPolicy = FetchPolicy.NoCache,
      ),
    );
    _stopCadence = effect(() {
      final entries = _indexing.data.value?.scannerStatus;
      final busy = entries?.any(_running) ?? false;
      untracked(() => _setCadence(busy));
    });
  }

  /// Quick while a scan runs, so its counts move; idly otherwise.
  void _setCadence(bool busy) {
    if (_pollingBusy == busy) return;
    _pollingBusy = busy;
    _poll?.cancel();
    _poll = Poller(() {
      _now.value = DateTime.now();
      _indexing.refetch();
    }, Duration(milliseconds: busy ? 2500 : 15000));
  }

  @override
  void dispose() {
    _stopCadence();
    _poll?.cancel();
    _indexing.dispose();
    _endpoint.dispose();
    _url.dispose();
    _apiKey.dispose();
    _model.dispose();
    _dimensions.dispose();
    _queryPrefix.dispose();
    _documentPrefix.dispose();
    super.dispose();
  }

  void _edit() {
    final current = _endpoint.data.value?.embeddingEndpoint;
    _url.text = current?.url ?? '';
    _apiKey.text = '';
    _model.text = current?.model ?? '';
    _dimensions.text = current?.requestDimensions?.toString() ?? '';
    _queryPrefix.text = current?.queryPrefix ?? '';
    _documentPrefix.text = current?.documentPrefix ?? '';
    _editing.value = true;
  }

  Future<void> _save() async {
    if (_url.text.trim().isEmpty || _model.text.trim().isEmpty) return;
    _saving.value = true;
    final saved = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GSaveEmbeddingEndpointReq(
          (b) => b.vars.input
            ..url = _url.text.trim()
            ..model = _model.text.trim()
            // An untouched key field keeps the saved key.
            ..apiKey = _apiKey.text.isEmpty ? null : _apiKey.text
            ..requestDimensions = int.tryParse(_dimensions.text.trim())
            ..queryPrefix = _queryPrefix.text
            ..documentPrefix = _documentPrefix.text,
        ),
      ),
      success: 'Embeddings API saved; reindexing',
    );
    if (!mounted) return;
    _saving.value = false;
    if (saved != null) {
      _editing.value = false;
      _endpoint.refetch();
      _indexing.refetch();
    }
  }

  Future<void> _useLocal() async {
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(GClearEmbeddingEndpointReq()),
      success: 'Embedding on this server again',
    );
    if (done == null || !mounted) return;
    _editing.value = false;
    _endpoint.refetch();
    _indexing.refetch();
  }

  Future<void> _scan(_Entry entry) async {
    _starting.value = entry.library.id;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GTriggerLibraryScanReq((b) => b.vars.id = entry.library.id),
      ),
      success: 'Scanning ${entry.library.name}',
    );
    if (!mounted) return;
    _starting.value = null;
    if (done != null) {
      _now.value = DateTime.now();
      _indexing.refetch();
      widget.controller.libraryContentChanged();
    }
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      if (_indexing.waiting.value) {
        return const NfSkeletonRows();
      }
      final data = _indexing.data.value;
      if (data == null) {
        return NfState.error(
          title: 'Couldn’t read indexing state',
          message: _indexing.error.value?.toString(),
          retry: _indexing.refetch,
        );
      }
      final entries = data.scannerStatus.toList();
      return AdminStack(
        children: [
          if (entries.isEmpty)
            const NfState(
              icon: PhosphorIconsRegular.books,
              title: 'No libraries',
              message: 'Create a library on the Libraries tab; its scans, watcher and schedule show up here.',
            ),
          for (final entry in entries) _entryPanel(entry),
          _searchPanel(data.searchIndexStatus),
        ],
      );
    },
  );

  Widget _entryPanel(_Entry entry) {
    final library = entry.library;
    final now = _now.value;
    final running = _running(entry);
    final last = _lastAt(entry);
    final next = nextScan(
      schedule: library.config.scanSchedule.name,
      last: last,
      now: now,
    );
    final startedAt =
        entry.activeScan?.startedAt ??
        DateTime.tryParse(entry.scan.startedAt ?? '');
    final (scanLabel, scanTone) = running
        ? (
            'Scanning ${startedAt == null ? '' : timecode(now.difference(startedAt).inMilliseconds)}',
            StateTone.accent,
          )
        : entry.scan.state == GScanState.FAILED
        ? ('Last scan failed', StateTone.bad)
        : last != null
        ? ('Indexed', StateTone.ok)
        : ('Never scanned', StateTone.none);
    final watcher = entry.watcher;
    final (
      watcherLabel,
      watcherTone,
      watcherDetail,
    ) = !library.config.watchEnabled
        ? (
            'Off',
            StateTone.none,
            'Filesystem watching is disabled for this library.',
          )
        : watcher.state == GWatcherState.ACTIVE
        ? (
            'Watching',
            StateTone.ok,
            watcher.since == null ? '' : 'Since ${stamp(watcher.since)}',
          )
        : watcher.state == GWatcherState.FAILED
        ? (
            'Failed',
            StateTone.bad,
            watcher.error ?? 'The watcher couldn’t start.',
          )
        : (
            'Not running',
            StateTone.warn,
            'Watching is enabled but no watcher is running.',
          );
    final failedStarted = _expanded.value.contains(library.id);
    final active = entry.activeScan;
    final done = entry.lastScan;
    return AccountPanel(
      icon: libraryKindIcon(library.kind),
      title: library.name,
      detail:
          '${titleCase(library.kind.name)} · ${plural(library.paths.length, 'folder')}',
      child: AdminStack(
        children: [
          Wrap(
            spacing: 8,
            runSpacing: 8,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              StatePill(scanLabel, tone: scanTone, busy: running),
              NfButton(
                label: 'Scan now',
                size: NfButtonSize.small,
                busy: _starting.value == library.id,
                icon: const Icon(PhosphorIconsRegular.arrowsClockwise),
                onPressed: running ? null : () => unawaited(_scan(entry)),
              ),
            ],
          ),
          Facts([
            Fact(
              'Schedule',
              scheduleLabels[library.config.scanSchedule.name] ??
                  library.config.scanSchedule.name,
            ),
            Fact(
              'Next scan',
              '',
              child: Tooltip(
                message: next.detail,
                child: Text(next.label, style: nfText(13.5, weight: 550)),
              ),
            ),
            Fact(
              'Last scan',
              last == null ? 'Never' : relative(last, now: now),
            ),
            Fact(
              'Watcher',
              '',
              child: Tooltip(
                message: watcherDetail,
                child: StatePill(watcherLabel, tone: watcherTone),
              ),
            ),
          ]),
          if (watcher.state == GWatcherState.FAILED && watcher.error != null)
            Notice(watcher.error!),
          if (entry.scan.state == GScanState.FAILED && entry.scan.error != null)
            Notice(
              'Scan failed ${relative(DateTime.tryParse(entry.scan.finishedAt ?? ''))}: ${entry.scan.error}',
            ),
          if (active != null)
            _RunCard(
              heading: 'Running now',
              counts:
                  '${active.added} added so far · ${active.failures} failed',
              live: true,
            ),
          if (done != null)
            _RunCard(
              heading: 'Last completed run',
              counts:
                  '${done.added} added · ${done.updated} updated · ${done.removed} removed · ${done.failures} failed',
              when: [
                stamp(done.finishedAt),
                if (done.startedAt != null && done.finishedAt != null)
                  'took ${timecode(done.finishedAt!.difference(done.startedAt!).inMilliseconds)}',
              ].join(' · '),
            ),
          Text('Folders', style: nfText(13.5, weight: 600)),
          Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final root in entry.roots)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Expanded(
                            child: SelectableText(
                              root.path,
                              style: NfType.mono,
                            ),
                          ),
                          const SizedBox(width: 8),
                          root.readable
                              ? const StatePill('Readable', tone: StateTone.ok)
                              : StatePill(
                                  root.exists
                                      ? (root.isDir
                                            ? 'Unreadable'
                                            : 'Not a folder')
                                      : 'Missing',
                                  tone: StateTone.bad,
                                ),
                        ],
                      ),
                      if (root.error != null && !root.readable)
                        Text(
                          root.error!,
                          style: nfText(12, color: NfColors.danger),
                        ),
                    ],
                  ),
                ),
            ],
          ),
          if (entry.failureCount > 0) ...[
            Row(
              children: [
                Expanded(
                  child: Text.rich(
                    TextSpan(
                      children: [
                        TextSpan(
                          text: 'Files that failed  ',
                          style: nfText(13.5, weight: 600),
                        ),
                        TextSpan(
                          text:
                              '${entry.failureCount} since the server started',
                          style: NfType.caption,
                        ),
                      ],
                    ),
                  ),
                ),
                NfButton(
                  label: failedStarted ? 'Hide' : 'Show',
                  style: NfButtonStyle.ghost,
                  size: NfButtonSize.small,
                  onPressed: () => _expanded.value = failedStarted
                      ? ({..._expanded.value}..remove(library.id))
                      : {..._expanded.value, library.id},
                ),
              ],
            ),
            if (failedStarted)
              for (final failed in entry.recentFailures)
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    SelectableText(
                      failed.path,
                      style: NfType.mono.copyWith(fontSize: 12),
                    ),
                    Text(
                      '${failed.stage.replaceFirst('scan: ', '')}${failed.error == null ? '' : ' — ${failed.error}'} · ${relative(failed.at)}',
                      style: NfType.caption,
                    ),
                  ],
                ),
          ],
        ],
      ),
    );
  }

  Widget _searchPanel(GAdminIndexingData_searchIndexStatus search) {
    final remote = search.remoteHost;
    final current = _endpoint.data.value?.embeddingEndpoint;
    final namespace = search.namespace;
    return AccountPanel(
      icon: PhosphorIconsRegular.magnifyingGlass,
      title: 'Search index',
      detail: remote != null
          ? 'Vector embeddings for hybrid search, computed by the API at $remote.'
          : 'Vector embeddings for hybrid search, computed on this server while it’s idle.',
      child: AdminStack(
        children: [
          Align(
            alignment: Alignment.centerLeft,
            child: namespace == null
                ? const StatePill('No embedder', tone: StateTone.warn)
                : search.pendingWorks > 0
                ? const StatePill('Backfilling', tone: StateTone.accent)
                : const StatePill('Up to date', tone: StateTone.ok),
          ),
          if (namespace != null)
            Facts([
              if (remote != null)
                Fact('Endpoint', remote, mono: true)
              else ...[
                Fact(
                  'Plugin',
                  search.embedderPlugins.isEmpty
                      ? '—'
                      : search.embedderPlugins.first,
                  mono: true,
                ),
                Fact(
                  'Running on',
                  search.inferenceDevice ?? 'Not loaded yet',
                  mono: true,
                ),
              ],
              Fact('Model', search.modelId ?? '—', mono: true),
              Fact('Namespace', namespace, mono: true),
              Fact('Dimensions', '${search.dimensions ?? '—'}'),
              Fact(
                'Waiting for vectors',
                '${search.pendingWorks}${search.pendingIsLowerBound ? '+' : ''} titles',
                color: search.pendingWorks > 0 ? NfColors.warning : null,
              ),
            ])
          else
            Text(
              'Search falls back to lexical matching. Install an embedder plugin or use an embeddings API to enable vector and hybrid search.',
              style: NfType.small,
            ),
          if (_editing.value)
            _endpointForm(current)
          else
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                NfButton(
                  label: current == null
                      ? 'Use an embeddings API'
                      : 'Edit embeddings API',
                  size: NfButtonSize.small,
                  onPressed: _edit,
                ),
                if (current != null)
                  NfButton(
                    label: 'Embed on this server instead',
                    style: NfButtonStyle.ghost,
                    size: NfButtonSize.small,
                    onPressed: () => unawaited(_useLocal()),
                  ),
              ],
            ),
        ],
      ),
    );
  }

  Widget _endpointForm(
    GEmbeddingEndpointSettingsData_embeddingEndpoint? current,
  ) => AdminStack(
    gap: 12,
    children: [
      const Notice(
        'An embeddings API receives every title, overview and credit in your libraries, and everything searched for. Saving sends a test, and reindexes the library if the model changes.',
        warn: true,
      ),
      NfField(
        controller: _url,
        label: 'Base URL',
        hint: 'https://api.openai.com/v1',
        autocorrect: false,
        keyboardType: TextInputType.url,
      ),
      NfField(
        controller: _model,
        label: 'Model',
        hint: 'text-embedding-3-small',
        autocorrect: false,
      ),
      Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          NfField(
            controller: _apiKey,
            label: 'API key',
            hint: current?.hasApiKey == true
                ? 'Saved; leave blank to keep'
                : 'None',
            obscure: true,
            autocorrect: false,
          ),
          const FieldNote(
            'A saved key is dropped if the URL changes, so it can’t reach another host.',
          ),
        ],
      ),
      Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          NfField(
            controller: _dimensions,
            label: 'Dimensions',
            hint: 'Model’s default',
            keyboardType: TextInputType.number,
          ),
          const FieldNote('Only for models that can shorten their vectors.'),
        ],
      ),
      Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          NfField(
            controller: _queryPrefix,
            label: 'Query prefix',
            hint: 'None',
            autocorrect: false,
          ),
          const FieldNote('Some models expect one, such as search_query: .'),
        ],
      ),
      NfField(
        controller: _documentPrefix,
        label: 'Document prefix',
        hint: 'None',
        autocorrect: false,
      ),
      Wrap(
        spacing: 8,
        children: [
          NfButton(
            label: 'Test and save',
            style: NfButtonStyle.primary,
            size: NfButtonSize.small,
            busy: _saving.value,
            onPressed: () => unawaited(_save()),
          ),
          NfButton(
            label: 'Cancel',
            style: NfButtonStyle.ghost,
            size: NfButtonSize.small,
            onPressed: () => _editing.value = false,
          ),
        ],
      ),
    ],
  );
}

class _RunCard extends StatelessWidget {
  const _RunCard({
    required this.heading,
    required this.counts,
    this.when,
    this.live = false,
  });

  final String heading;
  final String counts;
  final String? when;
  final bool live;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
    decoration: BoxDecoration(
      color: live ? NfColors.accentSoft : NfColors.surface,
      borderRadius: BorderRadius.circular(NfRadii.m),
      border: Border.all(color: live ? NfColors.accentSoft : NfColors.line),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(heading.toUpperCase(), style: NfType.eyebrow),
        const SizedBox(height: 4),
        Text(counts, style: nfText(13.5, color: NfColors.text2)),
        if (when != null) ...[
          const SizedBox(height: 2),
          Text(when!, style: NfType.caption),
        ],
      ],
    ),
  );
}
