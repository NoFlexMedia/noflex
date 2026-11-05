// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/create_library.req.gql.dart';
import '../../graphql/queries/__generated__/delete_library.req.gql.dart';
import '../../graphql/queries/__generated__/library_scan_status.req.gql.dart';
import '../../graphql/queries/__generated__/trigger_library_scan.req.gql.dart';
import '../../graphql/queries/__generated__/update_library.req.gql.dart';
import '../../graphql/queries/__generated__/update_library_config.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/admin_library_fields.data.gql.dart';
import '../../graphql/queries/fragments/__generated__/scan_status_fields.data.gql.dart';
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
import '../browse/library_screen.dart';
import 'admin_activity.dart';
import 'admin_controller.dart';
import 'admin_format.dart';
import 'admin_widgets.dart';

typedef AdminLibrary = GAdminLibraryFields;

const scheduleLabels = {
  'DISABLED': 'Manual only',
  'HOURLY': 'Hourly',
  'DAILY': 'Daily',
  'WEEKLY': 'Weekly',
};

String scanSummaryText(GScanStatusFields_summary? summary) {
  if (summary == null) return '';
  final errored = summary.errored > 0 ? ' · ${summary.errored} errors' : '';
  return '${summary.added} added · ${summary.updated} updated · ${summary.removed} removed$errored';
}

class LibrariesSection extends StatefulWidget {
  const LibrariesSection({super.key, required this.controller});
  final AdminController controller;

  @override
  State<LibrariesSection> createState() => _LibrariesSectionState();
}

class _LibrariesSectionState extends State<LibrariesSection> {
  final _scans = signal<Map<String, GScanStatusFields>>(const {});
  final _starting = signal<String?>(null);
  final _now = signal(DateTime.now());
  late final void Function() _stopKnown;
  Timer? _poll;

  AdminController get _controller => widget.controller;
  List<AdminLibrary> get _libraries =>
      _controller.admin.data.value?.libraries.toList() ?? const [];

  @override
  void initState() {
    super.initState();
    _stopKnown = effect(() {
      final libraries = _libraries;
      untracked(() {
        for (final library in libraries) {
          if (!_scans.value.containsKey(library.id)) {
            unawaited(_status(library));
          }
        }
      });
    });
  }

  @override
  void dispose() {
    _stopKnown();
    _poll?.cancel();
    super.dispose();
  }

  /// Polls and ticks the elapsed times only while something is scanning.
  void _schedule() {
    final running = _scans.value.values.any(
      (scan) => scan.state == GScanState.RUNNING,
    );
    if (!running) {
      _poll?.cancel();
      _poll = null;
      return;
    }
    _poll ??= Timer.periodic(const Duration(milliseconds: 2500), (_) {
      _now.value = DateTime.now();
      for (final library in _libraries) {
        if (_scans.value[library.id]?.state == GScanState.RUNNING) {
          unawaited(_status(library));
        }
      }
    });
  }

  Future<void> _status(AdminLibrary library) async {
    try {
      final data = await getIt<GraphqlService>().run(
        GLibraryScanStatusReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NoCache
            ..vars.id = library.id,
        ),
      );
      if (!mounted) return;
      _record(library, data.libraryScanStatus);
    } on Exception {
      // Status is decorative; the card still works without it.
    }
  }

  void _record(AdminLibrary library, GScanStatusFields next) {
    final previous = _scans.value[library.id];
    _scans.value = {..._scans.value, library.id: next};
    if (previous?.state == GScanState.RUNNING &&
        next.state != GScanState.RUNNING) {
      _finished(library, next);
    }
    _schedule();
  }

  void _finished(AdminLibrary library, GScanStatusFields scan) {
    _controller.libraryContentChanged();
    final toasts = getIt<ToastService>();
    if (scan.state == GScanState.FAILED) {
      toasts.show(
        'Scan of ${library.name} failed: ${scan.error ?? 'unknown error'}',
        kind: ToastKind.error,
      );
      return;
    }
    final added = scan.summary?.added ?? 0;
    toasts.show(
      '${library.name} is up to date${added > 0 ? ' — ${plural(added, 'new item')}' : ''}',
      kind: ToastKind.success,
    );
  }

  Future<void> _scan(AdminLibrary library) async {
    _starting.value = library.id;
    final data = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GTriggerLibraryScanReq((b) => b.vars.id = library.id),
      ),
    );
    if (!mounted) return;
    _starting.value = null;
    if (data == null) return;
    _now.value = DateTime.now();
    final scan = data.triggerLibraryScan;
    _scans.value = {..._scans.value, library.id: scan};
    if (scan.state != GScanState.RUNNING) _finished(library, scan);
    _schedule();
  }

  Future<void> _setWatching(AdminLibrary library, bool watching) async {
    final config = library.config;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GUpdateLibraryConfigReq(
          (b) => b.vars
            ..id = library.id
            ..languages.replace(config.preferredLanguages)
            ..providers.replace(config.preferredProviders)
            ..excluded.replace(config.excludedFolders)
            ..watching = watching
            ..schedule = config.scanSchedule,
        ),
      ),
      success: watching
          ? 'Watching ${library.name} for changes'
          : 'Stopped watching ${library.name}',
    );
    if (done != null) _controller.admin.refetch();
  }

  Future<void> _remove(AdminLibrary library) async {
    final confirmed = await confirmAction(
      title: 'Delete “${library.name}”?',
      message: 'Its titles, artwork and everyone’s watch history for it leave NoFlex. Media files stay on disk.',
      confirmLabel: 'Delete library',
      danger: true,
    );
    if (!confirmed) return;
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GDeleteLibraryReq((b) => b.vars.id = library.id),
      ),
      success: '${library.name} deleted',
    );
    if (done == null) return;
    _controller.admin.refetch();
    _controller.libraryContentChanged();
  }

  List<String> _providers() {
    final plugins = _controller.admin.data.value?.plugins;
    if (plugins == null) return const [];
    return [
      for (final plugin in plugins)
        if (plugin.hooks.contains(GHookKind.METADATA)) plugin.id,
    ];
  }

  Future<void> _openForm({AdminLibrary? library}) async {
    final result = await getIt<NavService>().openSheet<_FormResult>(
      (context) => LibraryForm(library: library, providers: _providers()),
      wide: true,
    );
    if (result == null || !mounted) return;
    _controller.admin.refetch();
    _controller.libraryContentChanged();
    if (result.scan) await _scan(result.library);
  }

  Future<void> _menu(AdminLibrary library, bool running) async {
    final action = await pickOption<String>(
      title: library.name,
      options: [
        const NfOption(
          'edit',
          'Edit library…',
          leading: Icon(PhosphorIconsRegular.pencilSimple, size: 18),
        ),
        const NfOption(
          'browse',
          'Browse',
          leading: Icon(PhosphorIconsRegular.arrowRight, size: 18),
        ),
        if (!running)
          const NfOption(
            'scan',
            'Scan now',
            leading: Icon(PhosphorIconsRegular.arrowsClockwise, size: 18),
          ),
        const NfOption(
          'delete',
          'Delete library',
          leading: Icon(
            PhosphorIconsRegular.trash,
            size: 18,
            color: NfColors.danger,
          ),
        ),
      ],
    );
    switch (action) {
      case 'edit':
        await _openForm(library: library);
      case 'browse':
        unawaited(
          getIt<NavService>().push(
            LibraryScreen(libraryId: library.id, title: library.name),
          ),
        );
      case 'scan':
        await _scan(library);
      case 'delete':
        await _remove(library);
    }
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final query = _controller.admin;
      final libraries = _libraries;
      if (query.waiting.value) {
        return const Padding(
          padding: EdgeInsets.all(40),
          child: Center(child: CircularProgressIndicator()),
        );
      }
      if (libraries.isEmpty) {
        return NfState(
          icon: PhosphorIconsRegular.books,
          title: 'No libraries yet',
          message: 'A library is a set of folders holding one kind of media — films, shows, music, audiobooks or photos.',
          actionLabel: 'Add your first library',
          onAction: () => unawaited(_openForm()),
        );
      }
      return AdminStack(
        children: [
          Align(
            alignment: Alignment.centerRight,
            child: NfButton(
              label: 'New library',
              style: NfButtonStyle.primary,
              size: NfButtonSize.small,
              icon: const Icon(PhosphorIconsRegular.plus),
              onPressed: () => unawaited(_openForm()),
            ),
          ),
          AdminColumns(
            minWidth: 460,
            children: [
              for (final library in libraries)
                _LibraryCard(
                  library: library,
                  scan: _scans.value[library.id],
                  starting: _starting.value == library.id,
                  now: _now.value,
                  onScan: () => unawaited(_scan(library)),
                  onEdit: () => unawaited(_openForm(library: library)),
                  onMenu: (running) => unawaited(_menu(library, running)),
                  onWatching: (value) =>
                      unawaited(_setWatching(library, value)),
                ),
            ],
          ),
        ],
      );
    },
  );
}

class _LibraryCard extends StatelessWidget {
  const _LibraryCard({
    required this.library,
    required this.scan,
    required this.starting,
    required this.now,
    required this.onScan,
    required this.onEdit,
    required this.onMenu,
    required this.onWatching,
  });

  final AdminLibrary library;
  final GScanStatusFields? scan;
  final bool starting;
  final DateTime now;
  final VoidCallback onScan;
  final VoidCallback onEdit;
  final ValueChanged<bool> onMenu;
  final ValueChanged<bool> onWatching;

  @override
  Widget build(BuildContext context) {
    final config = library.config;
    final state = scan?.state;
    final running = state == GScanState.RUNNING;
    final started = DateTime.tryParse(scan?.startedAt ?? '');
    final finished = DateTime.tryParse(scan?.finishedAt ?? '');
    return AccountPanel(
      icon: libraryKindIcon(library.kind),
      title: library.name,
      detail:
          '${titleCase(library.kind.name)} · ${plural(library.paths.length, 'folder')}',
      trailing: NfIconButton(
        icon: PhosphorIconsBold.dotsThree,
        label: 'Manage ${library.name}',
        size: 34,
        iconSize: 18,
        onPressed: () => onMenu(running),
      ),
      child: AdminStack(
        children: [
          if (running)
            StatePill(
              'Scanning ${started == null ? '' : timecode(now.difference(started).inMilliseconds)}',
              tone: StateTone.accent,
              busy: true,
            )
          else if (state == GScanState.FAILED)
            Notice('Scan failed: ${scan?.error ?? 'Unknown error'}')
          else if (state == GScanState.COMPLETED)
            Wrap(
              spacing: 8,
              runSpacing: 6,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                StatePill(
                  'Scanned ${relative(finished)}',
                  tone: StateTone.ok,
                  icon: PhosphorIconsRegular.checkCircle,
                ),
                if (scan?.summary != null)
                  Text(scanSummaryText(scan!.summary), style: NfType.caption),
              ],
            ),
          Facts([
            Fact('Folders', library.paths.join('\n'), mono: true, full: true),
            Fact(
              'Metadata',
              '',
              child: config.preferredProviders.isEmpty
                  ? Text('Default order', style: NfType.small)
                  : Wrap(
                      spacing: 4,
                      runSpacing: 4,
                      crossAxisAlignment: WrapCrossAlignment.center,
                      children: [
                        for (final (index, provider)
                            in config.preferredProviders.indexed) ...[
                          if (index > 0)
                            const Icon(
                              PhosphorIconsRegular.caretRight,
                              size: 13,
                              color: NfColors.text3,
                            ),
                          TagChip(provider, mono: true),
                        ],
                      ],
                    ),
            ),
            Fact(
              'Rescans',
              scheduleLabels[config.scanSchedule.name] ??
                  titleCase(config.scanSchedule.name),
            ),
            if (config.preferredLanguages.isNotEmpty)
              Fact('Languages', config.preferredLanguages.join(', ')),
            if (config.excludedFolders.isNotEmpty)
              Fact('Excluded', config.excludedFolders.join(', '), mono: true),
          ]),
          const Divider(height: 1),
          AccountToggle(
            title: 'Watch for changes',
            value: config.watchEnabled,
            onChanged: onWatching,
          ),
          Wrap(
            alignment: WrapAlignment.end,
            spacing: 8,
            runSpacing: 8,
            children: [
              NfButton(
                label: 'Edit',
                size: NfButtonSize.small,
                onPressed: onEdit,
              ),
              NfButton(
                label: running ? 'Scanning…' : 'Scan now',
                style: NfButtonStyle.primary,
                size: NfButtonSize.small,
                busy: running || starting,
                icon: const Icon(PhosphorIconsRegular.arrowsClockwise),
                onPressed: running || starting ? null : onScan,
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _FormResult {
  const _FormResult(this.library, {this.scan = false});
  final AdminLibrary library;
  final bool scan;
}

bool _same(Iterable<String> a, Iterable<String> b) {
  final left = a.toList();
  final right = b.toList();
  if (left.length != right.length) return false;
  for (var i = 0; i < left.length; i++) {
    if (left[i] != right[i]) return false;
  }
  return true;
}

/// Creating takes a dialog of its own; editing the same fields, seeded once so a
/// background refresh can't clobber what's been typed.
class LibraryForm extends StatefulWidget {
  const LibraryForm({super.key, required this.providers, this.library});
  final AdminLibrary? library;
  final List<String> providers;

  @override
  State<LibraryForm> createState() => _LibraryFormState();
}

class _LibraryFormState extends State<LibraryForm> {
  late final TextEditingController _name;
  late final TextEditingController _paths;
  late final TextEditingController _excluded;
  late final TextEditingController _languages;
  late final Signal<GLibraryKind> _kind;
  late final Signal<GScanSchedule> _schedule;
  late final Signal<bool> _watching;
  late final Signal<GArtworkPreference> _artwork;
  late final Signal<GLibraryCategory> _category;
  late final Signal<List<String>> _chosen;
  final _scan = signal(true);
  final _saving = signal(false);
  late final Signal<bool> _ready;

  @override
  void initState() {
    super.initState();
    final library = widget.library;
    final config = library?.config;
    _name = TextEditingController(text: library?.name ?? '');
    _paths = TextEditingController(text: library?.paths.join('\n') ?? '');
    _excluded = TextEditingController(
      text: config?.excludedFolders.join('\n') ?? '',
    );
    _languages = TextEditingController(
      text: config?.preferredLanguages.join(', ') ?? '',
    );
    _kind = signal(library?.kind ?? GLibraryKind.MOVIES);
    _schedule = signal(config?.scanSchedule ?? GScanSchedule.DAILY);
    _watching = signal(config?.watchEnabled ?? true);
    _artwork = signal(
      config?.artworkPreference ?? GArtworkPreference.PREFER_PROVIDER,
    );
    _category = signal(config?.contentCategory ?? GLibraryCategory.AUTO);
    _chosen = signal(config?.preferredProviders.toList() ?? const []);
    _ready = signal(_filled());
  }

  bool _filled() =>
      _name.text.trim().isNotEmpty && _paths.text.trim().isNotEmpty;

  @override
  void dispose() {
    _name.dispose();
    _paths.dispose();
    _excluded.dispose();
    _languages.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final paths = splitList(_paths.text);
    final languages = splitList(_languages.text);
    final excluded = splitList(_excluded.text);
    final providers = _chosen.value;
    final name = _name.text.trim();
    final graphql = getIt<GraphqlService>();
    final toasts = getIt<ToastService>();

    Future<AdminLibrary> writeConfig(String id) async {
      final data = await graphql.run(
        GUpdateLibraryConfigReq(
          (b) => b.vars
            ..id = id
            ..languages.replace(languages)
            ..providers.replace(providers)
            ..excluded.replace(excluded)
            ..watching = _watching.value
            ..schedule = _schedule.value
            ..artwork = _artwork.value
            ..category = _category.value,
        ),
      );
      return data.updateLibraryConfig;
    }

    _saving.value = true;
    final library = widget.library;
    AdminLibrary? result;
    if (library == null) {
      // Two steps. Once the library exists, a config failure mustn't invite a
      // duplicate create.
      final created = await toasts.attempt(
        () => graphql.run(
          GCreateLibraryReq(
            (b) => b.vars
              ..name = name
              ..kind = _kind.value
              ..paths.replace(paths),
          ),
        ),
        success: '“$name” created',
      );
      if (created != null) {
        result = created.createLibrary;
        result = await toasts.attempt(() => writeConfig(result!.id)) ?? result;
      }
    } else {
      result = await toasts.attempt(() async {
        AdminLibrary next = library;
        if (name != library.name ||
            _kind.value != library.kind ||
            !_same(paths, library.paths)) {
          next = (await graphql.run(
            GUpdateLibraryReq(
              (b) => b.vars
                ..id = library.id
                ..name = name
                ..kind = _kind.value
                ..paths.replace(paths),
            ),
          )).updateLibrary;
        }
        final config = library.config;
        if (!_same(languages, config.preferredLanguages) ||
            !_same(providers, config.preferredProviders) ||
            !_same(excluded, config.excludedFolders) ||
            _watching.value != config.watchEnabled ||
            _schedule.value != config.scanSchedule ||
            _artwork.value != config.artworkPreference ||
            _category.value != config.contentCategory) {
          next = await writeConfig(library.id);
        }
        return next;
      }, success: 'Library saved');
    }
    if (!mounted) return;
    _saving.value = false;
    if (result != null) {
      Navigator.of(context)
          .pop(_FormResult(result, scan: library == null && _scan.value));
    }
  }

  void _move(int index, int by) {
    final next = [..._chosen.value];
    final moved = next[index];
    next[index] = next[index + by];
    next[index + by] = moved;
    _chosen.value = next;
  }

  @override
  Widget build(BuildContext context) {
    final library = widget.library;
    return AdminSheet(
      title: library == null ? 'New library' : 'Edit “${library.name}”',
      description: library == null
          ? 'Point NoFlex at one or more folders of media.'
          : 'Folders, scanner behaviour and metadata providers.',
      footer: [
        NfButton(
          label: 'Cancel',
          style: NfButtonStyle.ghost,
          onPressed: () => Navigator.of(context).pop(),
        ),
        SignalBuilder(
          builder: (context) => NfButton(
            label: library == null ? 'Create library' : 'Save changes',
            style: NfButtonStyle.primary,
            busy: _saving.value,
            onPressed: _ready.value ? () => unawaited(_submit()) : null,
          ),
        ),
      ],
      child: SignalBuilder(
        builder: (context) {
          final chosen = _chosen.value;
          final available = [
            for (final id in widget.providers)
              if (!chosen.contains(id)) id,
          ];
          return AdminStack(
            gap: 12,
            children: [
              const FormHeading('Basics'),
              NfField(
                controller: _name,
                label: 'Name',
                hint: 'Films',
                autocorrect: false,
                onChanged: (_) => _ready.value = _filled(),
              ),
              AdminSelect<GLibraryKind>(
                label: 'Type',
                options: [
                  for (final kind in const [
                    GLibraryKind.MOVIES,
                    GLibraryKind.SHOWS,
                    GLibraryKind.MUSIC,
                    GLibraryKind.AUDIOBOOKS,
                    GLibraryKind.PHOTOS,
                    GLibraryKind.MIXED,
                  ])
                    NfOption(kind, titleCase(kind.name)),
                ],
                value: _kind.value,
                onChanged: (value) => _kind.value = value,
              ),
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  NfField(
                    controller: _paths,
                    label: 'Folders',
                    hint: '/srv/media/films\n/mnt/archive/films',
                    maxLines: 3,
                    autocorrect: false,
                    onChanged: (_) => _ready.value = _filled(),
                  ),
                  const FieldNote(
                    'One absolute path per line, as the server sees it.',
                  ),
                ],
              ),
              const SizedBox(height: 4),
              const FormHeading('Scanning'),
              AdminSelect<GScanSchedule>(
                label: 'Scheduled rescans',
                options: const [
                  NfOption(GScanSchedule.DISABLED, 'Manual only'),
                  NfOption(GScanSchedule.HOURLY, 'Every hour'),
                  NfOption(GScanSchedule.DAILY, 'Daily'),
                  NfOption(GScanSchedule.WEEKLY, 'Weekly'),
                ],
                value: _schedule.value,
                onChanged: (value) => _schedule.value = value,
              ),
              AccountToggle(
                title: 'Watch folders',
                detail: 'Pick up changes as they happen',
                value: _watching.value,
                onChanged: (value) => _watching.value = value,
              ),
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  NfField(
                    controller: _excluded,
                    label: 'Excluded folders',
                    hint: 'incoming/.staging\nfailed',
                    maxLines: 2,
                    autocorrect: false,
                  ),
                  const FieldNote(
                    'Relative to each folder above; skipped recursively.',
                  ),
                ],
              ),
              const SizedBox(height: 4),
              const FormHeading('Metadata'),
              Text(
                'Provider order',
                style: nfText(13, weight: 600, color: NfColors.text2),
              ),
              if (chosen.isEmpty)
                Text(
                  'None chosen: every installed provider is tried, in its default priority.',
                  style: NfType.small,
                )
              else
                Column(
                  children: [
                    for (final (index, id) in chosen.indexed)
                      Padding(
                        padding: const EdgeInsets.only(bottom: 4),
                        child: DecoratedBox(
                          decoration: BoxDecoration(
                            color: NfColors.surface,
                            borderRadius: BorderRadius.circular(NfRadii.s),
                            border: Border.all(color: NfColors.line),
                          ),
                          child: Row(
                            children: [
                              const SizedBox(width: 12),
                              Text('${index + 1}', style: NfType.caption),
                              const SizedBox(width: 10),
                              Expanded(
                                child: Tooltip(
                                  message: widget.providers.contains(id)
                                      ? ''
                                      : 'Not installed',
                                  child: Text(
                                    id,
                                    style: NfType.mono.copyWith(
                                      decoration: widget.providers.contains(id)
                                          ? null
                                          : TextDecoration.lineThrough,
                                    ),
                                  ),
                                ),
                              ),
                              NfIconButton(
                                icon: PhosphorIconsRegular.arrowUp,
                                label: 'Move $id up',
                                size: 34,
                                iconSize: 15,
                                onPressed: index == 0
                                    ? null
                                    : () => _move(index, -1),
                              ),
                              NfIconButton(
                                icon: PhosphorIconsRegular.arrowDown,
                                label: 'Move $id down',
                                size: 34,
                                iconSize: 15,
                                onPressed: index == chosen.length - 1
                                    ? null
                                    : () => _move(index, 1),
                              ),
                              NfIconButton(
                                icon: PhosphorIconsRegular.x,
                                label: 'Remove $id',
                                size: 34,
                                iconSize: 15,
                                onPressed: () => _chosen.value = [
                                  for (final other in chosen)
                                    if (other != id) other,
                                ],
                              ),
                            ],
                          ),
                        ),
                      ),
                  ],
                ),
              if (available.isNotEmpty)
                Wrap(
                  spacing: 6,
                  runSpacing: 6,
                  children: [
                    for (final id in available)
                      NfChoiceChip(
                        label: id,
                        selected: false,
                        leading: const Icon(PhosphorIconsBold.plus),
                        onTap: () => _chosen.value = [...chosen, id],
                      ),
                  ],
                ),
              const FieldNote(
                'Only these providers are used, in this order. The first that matches a title supplies all of its metadata; the next is tried only if it fails.',
              ),
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  NfField(
                    controller: _languages,
                    label: 'Preferred languages',
                    hint: 'en, ja',
                    autocorrect: false,
                  ),
                  const FieldNote('BCP-47 tags, most preferred first.'),
                ],
              ),
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  AdminSelect<GArtworkPreference>(
                    label: 'Artwork',
                    options: const [
                      NfOption(
                        GArtworkPreference.PREFER_PROVIDER,
                        'From metadata providers',
                      ),
                      NfOption(
                        GArtworkPreference.PREFER_LOCAL,
                        'From the media folders',
                      ),
                    ],
                    value: _artwork.value,
                    onChanged: (value) => _artwork.value = value,
                  ),
                  const FieldNote(
                    'Posters and backdrops to show first; the other source fills any gaps.',
                  ),
                ],
              ),
              Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  AdminSelect<GLibraryCategory>(
                    label: 'Anime',
                    options: const [
                      NfOption(GLibraryCategory.AUTO, 'Decide per title'),
                      NfOption(
                        GLibraryCategory.ANIME,
                        'Everything here is anime',
                      ),
                      NfOption(
                        GLibraryCategory.GENERAL,
                        'Nothing here is anime',
                      ),
                    ],
                    value: _category.value,
                    onChanged: (value) => _category.value = value,
                  ),
                  const FieldNote(
                    'Whether titles here appear under Anime or under Movies and TV Shows when browsing.',
                  ),
                ],
              ),
              if (library == null)
                AdminCheck(
                  label: 'Start the first scan right away',
                  value: _scan.value,
                  onChanged: (value) => _scan.value = value,
                ),
            ],
          );
        },
      ),
    );
  }
}
