// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/admin.data.gql.dart';
import '../../graphql/queries/__generated__/admin_plugin_events.data.gql.dart';
import '../../graphql/queries/__generated__/admin_plugin_events.req.gql.dart';
import '../../graphql/queries/__generated__/clear_plugin_override.req.gql.dart';
import '../../graphql/queries/__generated__/enqueue_plugin_task.req.gql.dart';
import '../../graphql/queries/__generated__/set_plugin_override.req.gql.dart';
import '../../graphql/query.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../account/account_widgets.dart';
import 'admin_controller.dart';
import 'admin_format.dart';
import 'admin_widgets.dart';

typedef _Plugin = GAdminData_plugins;
typedef _Override = GAdminData_pluginOverrides;
typedef _Event = GAdminPluginEventsData_pluginEvents_nodes;

final _allHooks = GHookKind.values.toList();
const _workKinds = [
  GWorkKindFilter.MOVIE,
  GWorkKindFilter.SERIES,
  GWorkKindFilter.SEASON,
  GWorkKindFilter.EPISODE,
  GWorkKindFilter.ALBUM,
  GWorkKindFilter.TRACK,
  GWorkKindFilter.ARTIST,
  GWorkKindFilter.AUDIO_BOOK,
  GWorkKindFilter.PHOTO,
];

const _unloadReasons = {
  'FileRemoved': 'file removed',
  'ReplacedByHotReload': 'replaced by hot reload',
  'TooManyTraps': 'too many crashes',
  'AdminRequested': 'unloaded by an admin',
  'Shutdown': 'server shutdown',
};

String hookLabel(GHookKind hook) => hook == GHookKind.POST_SCAN
    ? 'Post-scan'
    : hook == GHookKind.USER_LINK
    ? 'Account link'
    : titleCase(hook.name);

/// How a plugin is doing, in order of what matters most.
({StateTone tone, String label, String detail}) pluginHealth({
  required bool circuitOpen,
  required bool taskFailing,
  required int restarts,
}) {
  if (circuitOpen) {
    return (
      tone: StateTone.bad,
      label: 'Circuit open',
      detail: 'Disabled after repeated crashes — reload the plugin to recover.',
    );
  }
  if (taskFailing) {
    return (
      tone: StateTone.warn,
      label: 'Task failing',
      detail: 'A background task failed on its last run.',
    );
  }
  if (restarts > 0) {
    return (
      tone: StateTone.warn,
      label: 'Recovered',
      detail: 'Restarted ${plural(restarts, 'time')} after a crash.',
    );
  }
  return (tone: StateTone.ok, label: 'Healthy', detail: '');
}

class PluginsSection extends StatefulWidget {
  const PluginsSection({super.key, required this.controller});
  final AdminController controller;

  @override
  State<PluginsSection> createState() => _PluginsSectionState();
}

class _PluginsSectionState extends State<PluginsSection> {
  late final Pager<_Event> _events;
  Poller? _eventsPoll;
  final _plugin = signal<String?>(null);
  final _hook = signal<GHookKind?>(null);
  final _kind = signal(GWorkKindFilter.MOVIE);
  final _priority = TextEditingController(text: '100');
  final _saving = signal(false);
  Timer? _delayedRefetch;

  @override
  void initState() {
    super.initState();
    _events = Pager((after) async {
      final data = await getIt<GraphqlService>().run(
        GAdminPluginEventsReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NoCache
            ..vars.after = after,
        ),
      );
      final info = data.pluginEvents.pageInfo;
      return (
        data.pluginEvents.nodes.toList(),
        info.hasNextPage ? info.endCursor : null,
      );
    });
    unawaited(_events.refresh());
    _eventsPoll = Poller(
      () => unawaited(_events.refresh()),
      const Duration(seconds: 10),
    );
  }

  @override
  void dispose() {
    _eventsPoll?.cancel();
    _delayedRefetch?.cancel();
    _priority.dispose();
    super.dispose();
  }

  Future<void> _olderEvents() async {
    // Polling refetches the first page, which would drop the older ones.
    _eventsPoll?.cancel();
    _eventsPoll = null;
    await _events.loadMore();
  }

  Future<void> _reset(_Override row, String name) async {
    final confirmed = await confirmAction(
      title: 'Reset $name’s ${hookLabel(row.hook)} priority?',
      message:
          'The admin-set priority for ${titleCase(row.workKind.name)} goes away and the manifest or config default applies again.',
      confirmLabel: 'Reset',
    );
    if (!confirmed) return;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GClearPluginOverrideReq(
          (b) => b.vars
            ..plugin = row.pluginId
            ..hook = row.hook
            ..kind = row.workKind,
        ),
      ),
      success: 'Priority reset to default',
    );
    if (done != null) widget.controller.admin.refetch();
  }

  Future<void> _saveOverride() async {
    final plugin = _plugin.value;
    final hook = _hook.value;
    final priority = int.tryParse(_priority.text.trim());
    if (plugin == null || hook == null || priority == null) return;
    _saving.value = true;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GSetPluginOverrideReq(
          (b) => b.vars
            ..plugin = plugin
            ..hook = hook
            ..kind = _kind.value
            ..priority = priority,
        ),
      ),
      success: 'Priority override saved',
    );
    if (!mounted) return;
    _saving.value = false;
    if (done != null) widget.controller.admin.refetch();
  }

  void _edit(_Override row) {
    batch(() {
      _plugin.value = row.pluginId;
      _hook.value = row.hook;
      _kind.value = row.workKind;
    });
    _priority.text = '${row.priority}';
  }

  Future<void> _runTask(_Plugin plugin, String task) async {
    final ran = await getIt<NavService>().openSheet<bool>(
      (context) => _RunTaskSheet(plugin: plugin, task: task),
    );
    if (ran == true && mounted) {
      _delayedRefetch?.cancel();
      _delayedRefetch = Timer(
        const Duration(milliseconds: 1500),
        widget.controller.admin.refetch,
      );
    }
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final controller = widget.controller;
      if (controller.admin.waiting.value) {
        return const Padding(
          padding: EdgeInsets.all(40),
          child: Center(child: CircularProgressIndicator()),
        );
      }
      final data = controller.admin.data.value;
      final plugins = (data?.plugins.toList() ?? <_Plugin>[])
        ..sort((a, b) {
          if (a.circuitOpen != b.circuitOpen) return a.circuitOpen ? -1 : 1;
          return a.displayName.compareTo(b.displayName);
        });
      final overrides = (data?.pluginOverrides.toList() ?? <_Override>[])
        ..sort((a, b) {
          final byHook = a.hook.name.compareTo(b.hook.name);
          if (byHook != 0) return byHook;
          final byKind = a.workKind.name.compareTo(b.workKind.name);
          return byKind != 0 ? byKind : b.priority.compareTo(a.priority);
        });
      final names = {
        for (final plugin in plugins) plugin.id: plugin.displayName,
      };
      final health = controller.live.status.value?.plugins;
      return AdminStack(
        children: [
          if (plugins.isEmpty)
            const NfPanel(
              child: NfState(
                icon: PhosphorIconsRegular.plug,
                title: 'No plugins loaded',
                message: 'Plugins are WASM components that add metadata providers, sign-in methods, scrobblers and automation. Drop a .wasm into the server’s plugin folder and it loads automatically.',
              ),
            )
          else ...[
            if (health != null)
              Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  StatePill(
                    '${health.loaded - health.circuitOpen} healthy',
                    tone: StateTone.ok,
                  ),
                  if (health.circuitOpen > 0)
                    StatePill(
                      '${health.circuitOpen} disabled',
                      tone: StateTone.bad,
                    ),
                  StatePill(
                    '${health.instanceRestarts} restarts',
                    tone: health.instanceRestarts > 0
                        ? StateTone.warn
                        : StateTone.none,
                  ),
                  StatePill(
                    '${health.traps} traps since start',
                    tone: health.traps > 0 ? StateTone.bad : StateTone.none,
                  ),
                  StatePill(
                    '${health.loadFailures} load failures',
                    tone: health.loadFailures > 0
                        ? StateTone.bad
                        : StateTone.none,
                  ),
                ],
              ),
            AdminColumns(
              minWidth: 380,
              children: [
                for (final plugin in plugins)
                  _PluginCard(plugin: plugin, onRun: _runTask),
              ],
            ),
            AccountPanel(
              icon: PhosphorIconsRegular.arrowsDownUp,
              title: 'Priority overrides',
              detail: 'Decide which plugin runs first for a hook and type of title. Higher priority wins; admin-set rows can be reset to the manifest or config default.',
              child: AdminStack(
                children: [
                  AdminTable(
                    minWidth: 600,
                    empty: 'No overrides — plugins run in each library’s provider order.',
                    columns: const [
                      AdminCol('Plugin', flex: true),
                      AdminCol('Hook'),
                      AdminCol('Applies to'),
                      AdminCol('Set by'),
                      AdminCol('Priority', numeric: true),
                      AdminCol(''),
                    ],
                    rows: [
                      for (final row in overrides)
                        [
                          cellText(
                            names[row.pluginId] ?? row.pluginId,
                            strong: true,
                          ),
                          cellText(hookLabel(row.hook)),
                          cellText(titleCase(row.workKind.name)),
                          row.adminSet
                              ? const StatePill('Admin', tone: StateTone.accent)
                              : cellText('Default', color: NfColors.text3),
                          cellText('${row.priority}'),
                          Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              NfIconButton(
                                icon: PhosphorIconsRegular.pencilSimple,
                                label:
                                    'Edit override for ${names[row.pluginId] ?? row.pluginId}',
                                size: 32,
                                iconSize: 15,
                                onPressed: () => _edit(row),
                              ),
                              if (row.adminSet)
                                NfIconButton(
                                  icon: PhosphorIconsRegular
                                      .clockCounterClockwise,
                                  label:
                                      'Reset ${names[row.pluginId] ?? row.pluginId} to its default priority',
                                  size: 32,
                                  iconSize: 15,
                                  onPressed: () => unawaited(
                                    _reset(
                                      row,
                                      names[row.pluginId] ?? row.pluginId,
                                    ),
                                  ),
                                ),
                            ],
                          ),
                        ],
                    ],
                  ),
                  const Divider(height: 1),
                  _overrideForm(plugins),
                ],
              ),
            ),
          ],
          AccountPanel(
            icon: PhosphorIconsRegular.clockCounterClockwise,
            title: 'Lifecycle events',
            detail: 'Discovery, loads, load failures, unloads and crashes since the server started, newest first.',
            child: _eventList(names),
          ),
        ],
      );
    },
  );

  Widget _overrideForm(List<_Plugin> plugins) {
    final chosen = _plugin.value;
    _Plugin? plugin;
    for (final item in plugins) {
      if (item.id == chosen) plugin = item;
    }
    final hooks = plugin?.hooks.toList() ?? _allHooks;
    final hook = hooks.contains(_hook.value) ? _hook.value : null;
    return AdminColumns(
      minWidth: 200,
      gap: 10,
      children: [
        AdminSelect<String>(
          label: 'Plugin',
          options: [
            for (final item in plugins)
              NfOption(item.id, item.displayName, detail: item.id),
          ],
          value: chosen,
          onChanged: (value) => _plugin.value = value,
        ),
        AdminSelect<GHookKind>(
          label: 'Hook',
          options: [for (final item in hooks) NfOption(item, hookLabel(item))],
          value: hook,
          onChanged: (value) => _hook.value = value,
        ),
        AdminSelect<GWorkKindFilter>(
          label: 'Applies to',
          options: [
            for (final kind in _workKinds) NfOption(kind, titleCase(kind.name)),
          ],
          value: _kind.value,
          onChanged: (value) => _kind.value = value,
        ),
        NfField(
          controller: _priority,
          label: 'Priority',
          keyboardType: const TextInputType.numberWithOptions(signed: true),
          autocorrect: false,
        ),
        Padding(
          padding: const EdgeInsets.only(top: 25),
          child: NfButton(
            label: 'Save',
            style: NfButtonStyle.primary,
            busy: _saving.value,
            onPressed: chosen == null || hook == null
                ? null
                : () => unawaited(_saveOverride()),
          ),
        ),
      ],
    );
  }

  Widget _eventList(Map<String, String> names) {
    final events = _events.items.value;
    if (events.isEmpty) {
      return Text(
        _events.loading.value
            ? 'Loading…'
            : 'No lifecycle events recorded since the server started.',
        style: NfType.caption,
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final (index, node) in events.indexed)
          Container(
            padding: const EdgeInsets.symmetric(vertical: 9),
            decoration: BoxDecoration(
              border: index == 0
                  ? null
                  : const Border(top: BorderSide(color: NfColors.line)),
            ),
            child: _EventRow(node: node, names: names),
          ),
        if (_events.hasMore.value)
          Padding(
            padding: const EdgeInsets.only(top: 12),
            child: Center(
              child: NfButton(
                label: 'Load older',
                size: NfButtonSize.small,
                busy: _events.loading.value,
                onPressed: () => unawaited(_olderEvents()),
              ),
            ),
          ),
      ],
    );
  }
}

class _EventRow extends StatelessWidget {
  const _EventRow({required this.node, required this.names});
  final _Event node;
  final Map<String, String> names;

  @override
  Widget build(BuildContext context) {
    final event = node.event;
    final kind = event.kind;
    final (label, tone) = switch (kind) {
      GPluginLifecycleKind.DISCOVERED => ('Discovered', StateTone.info),
      GPluginLifecycleKind.LOADED => ('Loaded', StateTone.ok),
      GPluginLifecycleKind.LOAD_FAILED => ('Load failed', StateTone.bad),
      GPluginLifecycleKind.UNLOADED => ('Unloaded', StateTone.none),
      GPluginLifecycleKind.TRAPPED => ('Trapped', StateTone.bad),
      _ => (kind.name, StateTone.none),
    };
    final who =
        (event.pluginId == null
            ? null
            : names[event.pluginId] ?? event.pluginId) ??
        (event.path?.split('/').last ?? '');
    final hooks = event.hooks.toList();
    final detail = switch (kind) {
      GPluginLifecycleKind.TRAPPED =>
        'in ${hooks.isEmpty ? '' : hookLabel(hooks.first)} hook: ${event.trap ?? ''}',
      GPluginLifecycleKind.LOAD_FAILED =>
        '${event.path ?? ''} — ${event.reason ?? ''}',
      GPluginLifecycleKind.UNLOADED =>
        _unloadReasons[event.reason] ?? event.reason ?? '',
      _ =>
        hooks.isEmpty
            ? ''
            : [
                hooks.map(hookLabel).join(', '),
                if (event.sha256 != null)
                  event.sha256!.substring(
                    0,
                    event.sha256!.length < 12 ? event.sha256!.length : 12,
                  ),
              ].join(' · '),
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Wrap(
          spacing: 10,
          runSpacing: 4,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            Tooltip(
              message: stamp(node.at),
              child: Text(relative(node.at), style: NfType.caption),
            ),
            StatePill(label, tone: tone),
            Text(who, style: nfText(13.5, weight: 600)),
          ],
        ),
        if (detail.isNotEmpty) ...[
          const SizedBox(height: 3),
          Text(detail, style: NfType.caption),
        ],
      ],
    );
  }
}

class _PluginCard extends StatelessWidget {
  const _PluginCard({required this.plugin, required this.onRun});
  final _Plugin plugin;
  final Future<void> Function(_Plugin, String) onRun;

  @override
  Widget build(BuildContext context) {
    final vital = pluginHealth(
      circuitOpen: plugin.circuitOpen,
      taskFailing: plugin.tasks.any((task) => task.consecutiveFailures > 0),
      restarts: plugin.instanceRestarts,
    );
    final homepage = plugin.homepage;
    final isWeb =
        homepage != null &&
        (homepage.startsWith('http://') || homepage.startsWith('https://'));
    return AccountPanel(
      icon: PhosphorIconsRegular.plug,
      title: plugin.displayName,
      detail: '${plugin.id} · v${plugin.version}',
      trailing: StatePill(vital.label, tone: vital.tone),
      child: AdminStack(
        children: [
          if (vital.detail.isNotEmpty)
            Text(
              vital.detail,
              style: nfText(
                13,
                color: plugin.circuitOpen ? NfColors.danger : NfColors.warning,
              ),
            ),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              for (final hook in plugin.hooks) TagChip(hookLabel(hook)),
            ],
          ),
          Facts([
            Fact('Plugin API', plugin.apiVersion, mono: true),
            Fact(
              'Restarts',
              '${plugin.instanceRestarts}',
              color: plugin.instanceRestarts > 0 ? NfColors.warning : null,
            ),
            if (plugin.author != null) Fact('Author', plugin.author!),
            if (plugin.license != null) Fact('License', plugin.license!),
            Fact(
              'Capabilities',
              '',
              full: true,
              child: plugin.capabilities.isEmpty
                  ? Text('None requested', style: NfType.small)
                  : Wrap(
                      spacing: 5,
                      runSpacing: 5,
                      children: [
                        for (final capability in plugin.capabilities)
                          TagChip(capability, mono: true),
                      ],
                    ),
            ),
          ], minWidth: 110),
          if (plugin.tasks.isNotEmpty) ...[
            const Divider(height: 1),
            Text('BACKGROUND TASKS', style: NfType.eyebrow),
            for (final task in plugin.tasks)
              Row(
                children: [
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(task.name, style: NfType.mono),
                        Text.rich(
                          TextSpan(
                            style: NfType.caption,
                            children: [
                              TextSpan(
                                text:
                                    '${interval(task.intervalMs)} · last run ${relative(task.lastRunAt)} · ${plural(task.totalRuns, 'run')}',
                              ),
                              if (task.consecutiveFailures > 0)
                                TextSpan(
                                  text:
                                      ' · ${task.consecutiveFailures} failing',
                                  style: nfText(12, color: NfColors.warning),
                                ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                  const SizedBox(width: 8),
                  NfButton(
                    label: 'Run now',
                    style: NfButtonStyle.ghost,
                    size: NfButtonSize.small,
                    icon: const Icon(PhosphorIconsFill.play),
                    onPressed: plugin.circuitOpen
                        ? null
                        : () => unawaited(onRun(plugin, task.name)),
                  ),
                ],
              ),
          ],
          // Nothing in the app opens links, so the address is there to copy.
          if (isWeb)
            InkWell(
              borderRadius: BorderRadius.circular(NfRadii.xs),
              onTap: () => unawaited(_copyHomepage(homepage)),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const Icon(
                    PhosphorIconsRegular.copy,
                    size: 14,
                    color: NfColors.text2,
                  ),
                  const SizedBox(width: 6),
                  Flexible(
                    child: Text(
                      homepage,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: nfText(13, color: NfColors.text2),
                    ),
                  ),
                ],
              ),
            ),
        ],
      ),
    );
  }

  Future<void> _copyHomepage(String url) async {
    await Clipboard.setData(ClipboardData(text: url));
    getIt<ToastService>().show('Homepage address copied');
  }
}

class _RunTaskSheet extends StatefulWidget {
  const _RunTaskSheet({required this.plugin, required this.task});
  final _Plugin plugin;
  final String task;

  @override
  State<_RunTaskSheet> createState() => _RunTaskSheetState();
}

class _RunTaskSheetState extends State<_RunTaskSheet> {
  final _payload = TextEditingController();
  final _running = signal(false);

  @override
  void dispose() {
    _payload.dispose();
    super.dispose();
  }

  Future<void> _run() async {
    _running.value = true;
    final payload = _payload.text.trim();
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GEnqueuePluginTaskReq(
          (b) => b.vars
            ..plugin = widget.plugin.id
            ..task = widget.task
            ..payload = payload.isEmpty ? null : payload,
        ),
      ),
      success: '${widget.task} queued on ${widget.plugin.displayName}',
    );
    if (!mounted) return;
    _running.value = false;
    if (done != null) Navigator.of(context).pop(true);
  }

  @override
  Widget build(BuildContext context) => AdminSheet(
    title: 'Run ${widget.task} now',
    description:
        'Queues one extra run of ${widget.plugin.displayName}’s task, outside its regular schedule.',
    footer: [
      NfButton(
        label: 'Cancel',
        style: NfButtonStyle.ghost,
        onPressed: () => Navigator.of(context).pop(false),
      ),
      SignalBuilder(
        builder: (context) => NfButton(
          label: 'Run now',
          style: NfButtonStyle.primary,
          busy: _running.value,
          icon: const Icon(PhosphorIconsFill.play),
          onPressed: () => unawaited(_run()),
        ),
      ),
    ],
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        NfField(
          controller: _payload,
          label: 'Payload',
          hint: 'Optional — a string or JSON the task understands',
          maxLines: 4,
          autocorrect: false,
        ),
        const FieldNote('Leave empty for a normal run.'),
      ],
    ),
  );
}
