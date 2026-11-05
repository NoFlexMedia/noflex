// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/queries/__generated__/set_downloads_allowed.req.gql.dart';
import '../../services/graphql.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../account/account_widgets.dart';
import 'admin_controller.dart';
import 'admin_format.dart';
import 'admin_widgets.dart';

class ServerSection extends StatefulWidget {
  const ServerSection({super.key, required this.controller});
  final AdminController controller;

  @override
  State<ServerSection> createState() => _ServerSectionState();
}

class _ServerSectionState extends State<ServerSection> {
  final _filter = TextEditingController();
  final _needle = signal('');
  final _copied = signal(false);
  Timer? _copiedReset;

  @override
  void dispose() {
    _copiedReset?.cancel();
    _filter.dispose();
    super.dispose();
  }

  Future<void> _setDownloads(bool allowed) async {
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GSetDownloadsAllowedReq((b) => b.vars.allowed = allowed),
      ),
      success: allowed ? 'Downloads allowed' : 'Downloads turned off',
    );
    if (done != null) widget.controller.admin.refetch();
  }

  Future<void> _copy(SystemStatus status) async {
    final text = status.server.config
        .map((row) => '${row.key} = ${row.value ?? '(unset)'}')
        .join('\n');
    await Clipboard.setData(ClipboardData(text: text));
    _copied.value = true;
    _copiedReset?.cancel();
    _copiedReset = Timer(
      const Duration(milliseconds: 1600),
      () => _copied.value = false,
    );
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final status = widget.controller.live.status.value;
      if (status == null) {
        return const Padding(
          padding: EdgeInsets.all(40),
          child: Center(child: CircularProgressIndicator()),
        );
      }
      final server = status.server;
      final database = server.database;
      final host = status.host;
      final needle = _needle.value.trim().toLowerCase();
      final config = [
        for (final row in server.config)
          if (needle.isEmpty ||
              row.key.toLowerCase().contains(needle) ||
              (row.value ?? '').toLowerCase().contains(needle))
            row,
      ];
      final redacted = server.config.where((row) => row.redacted).length;
      final serverName = getIt<SessionService>().serverName.value;
      final info = widget.controller.admin.data.value?.serverInfo;
      return AdminStack(
        children: [
          if (info != null)
            AccountPanel(
              icon: PhosphorIconsRegular.downloadSimple,
              title: 'Downloads',
              detail: 'Whether people signed in here, or linked from another server, can keep copies on their devices. Streaming isn’t affected, and copies already on a device stay there.',
              child: AccountToggle(
                title: 'Allow downloads',
                value: info.downloadsAllowed,
                onChanged: (value) => unawaited(_setDownloads(value)),
              ),
            ),
          AdminColumns(
            children: [
              AccountPanel(
                icon: PhosphorIconsRegular.hardDrives,
                title: 'NoFlex ${server.version}',
                detail:
                    'Up ${uptime(server.uptimeSeconds)} · started ${stamp(server.startedAt)}',
                child: Facts([
                  Fact('Listening on', server.bind, mono: true),
                  Fact('Public URL', server.publicUrl ?? 'not set', mono: true),
                  Fact('Data folder', server.dataDir, mono: true, full: true),
                  Fact('Cache folder', server.cacheDir, mono: true, full: true),
                  if (serverName != null) Fact('Server name', serverName),
                ]),
              ),
              AccountPanel(
                icon: PhosphorIconsRegular.stack,
                title: 'Database',
                detail:
                    '${database.backend.isEmpty ? 'Storage' : database.backend} engine ${database.engineVersion}',
                trailing: StatePill(
                  database.error != null
                      ? 'Unreachable'
                      : database.healthy
                      ? 'Healthy'
                      : 'Unhealthy',
                  tone: database.healthy && database.error == null
                      ? StateTone.ok
                      : StateTone.bad,
                ),
                child: AdminStack(
                  children: [
                    if (database.error != null) Notice(database.error!),
                    Facts([
                      Fact('Schema version', '${database.schemaVersion}'),
                      Fact('Database file', size(database.sizeBytes)),
                      Fact('Write-ahead log', size(database.walSizeBytes)),
                      Fact(
                        'Path',
                        database.path ?? '—',
                        mono: true,
                        full: true,
                      ),
                      if (database.notes.isNotEmpty)
                        Fact('Notes', database.notes.join(' · '), full: true),
                    ]),
                  ],
                ),
              ),
              AccountPanel(
                icon: PhosphorIconsRegular.devices,
                title: 'Host',
                detail: host.hostname ?? 'Unknown host',
                child: Facts([
                  Fact('OS', host.osVersion ?? host.osName ?? '—'),
                  Fact('Kernel', host.kernelVersion ?? '—', mono: true),
                  Fact('Architecture', host.arch, mono: true),
                  Fact('Booted', relative(host.bootedAt)),
                ]),
              ),
            ],
          ),
          AccountPanel(
            icon: PhosphorIconsRegular.gear,
            title: 'Configuration',
            detail:
                'The resolved config — files plus NOFLEX_* overrides. ${redacted > 0 ? '$redacted secret-looking ${redacted == 1 ? 'value is' : 'values are'} hidden.' : 'Secret-looking values are hidden.'}',
            child: AdminStack(
              children: [
                Row(
                  children: [
                    Expanded(
                      child: NfField(
                        controller: _filter,
                        hint: 'Filter keys or values',
                        autocorrect: false,
                        onChanged: (value) => _needle.value = value,
                      ),
                    ),
                    const SizedBox(width: 8),
                    NfButton(
                      label: _copied.value ? 'Copied' : 'Copy',
                      style: NfButtonStyle.ghost,
                      size: NfButtonSize.small,
                      icon: Icon(
                        _copied.value
                            ? PhosphorIconsRegular.check
                            : PhosphorIconsRegular.copy,
                      ),
                      onPressed: () => unawaited(_copy(status)),
                    ),
                  ],
                ),
                AdminTable(
                  minWidth: 560,
                  empty: 'No keys match.',
                  columns: const [
                    AdminCol('Key'),
                    AdminCol('Value', flex: true),
                  ],
                  rows: [
                    for (final row in config)
                      [
                        cellMono(row.key),
                        if (row.redacted)
                          const StatePill(
                            'hidden',
                            tone: StateTone.warn,
                            icon: PhosphorIconsRegular.lock,
                          )
                        else if (row.value == null)
                          cellText('unset', color: NfColors.text3)
                        else if (row.value!.isEmpty)
                          cellText('empty', color: NfColors.text3)
                        else
                          cellMono(row.value!),
                      ],
                  ],
                ),
              ],
            ),
          ),
        ],
      );
    },
  );
}
