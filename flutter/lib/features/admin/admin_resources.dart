// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/fragments/__generated__/system_status_fields.data.gql.dart';
import '../../theme.dart';
import '../account/account_widgets.dart';
import 'admin_controller.dart';
import 'admin_format.dart';
import 'admin_overview.dart';
import 'admin_widgets.dart';

const _storageKinds = {
  'DATA': 'Data',
  'CACHE': 'Cache',
  'TRANSCODE_CACHE': 'Transcode cache',
  'IMAGE_CACHE': 'Image cache',
  'EMBED_CACHE': 'Embedding cache',
  'RECORDINGS': 'Recordings',
  'LIBRARY_ROOT': 'Library',
};

class ResourcesSection extends StatefulWidget {
  const ResourcesSection({super.key, required this.live});
  final LiveStatus live;

  @override
  State<ResourcesSection> createState() => _ResourcesSectionState();
}

class _ResourcesSectionState extends State<ResourcesSection> {
  final _showAllNics = signal(false);

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final live = widget.live;
      final status = live.status.value;
      final history = live.history.value;
      if (status == null) {
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            LiveBadge(live: live),
            const Padding(
              padding: EdgeInsets.all(40),
              child: Center(child: CircularProgressIndicator()),
            ),
          ],
        );
      }
      final memory = status.memory;
      final limit = memory.cgroupLimitBytes ?? memory.totalBytes;
      final host = status.host;
      final process = status.process;
      final load = status.cpu.loadAverage;
      final showAll = _showAllNics.value;
      final nics = [
        for (final nic in status.network)
          if (showAll ||
              (!nic.loopback &&
                  (nic.state == 'up' ||
                      nic.rxTotalBytes > 0 ||
                      nic.txTotalBytes > 0)))
            nic,
      ];
      final hidden = status.network.length - nics.length;
      return AdminStack(
        children: [
          Wrap(
            spacing: 12,
            runSpacing: 8,
            crossAxisAlignment: WrapCrossAlignment.center,
            children: [
              LiveBadge(live: live),
              Text(
                '${host.hostname} · ${host.osVersion ?? host.osName} · kernel ${host.kernelVersion ?? '?'} · ${host.arch} · host up ${uptime(host.uptimeSeconds)}',
                style: NfType.caption,
              ),
            ],
          ),
          AdminColumns(
            children: [
              AccountPanel(
                icon: PhosphorIconsRegular.squaresFour,
                title: 'CPU',
                detail:
                    '${status.cpu.brand.isEmpty ? 'Processor' : status.cpu.brand} · ${status.cpu.physicalCores ?? '?'} cores / ${status.cpu.logicalCores} threads',
                trailing: _Big(percent(status.cpu.usagePercent)),
                child: AdminStack(
                  children: [
                    Sparkline(
                      values: [for (final h in history) h.cpuPercent],
                      max: 100,
                      height: 54,
                      label: 'CPU usage',
                    ),
                    if (load != null)
                      Facts([
                        Fact('Load 1 min', load.one.toStringAsFixed(2)),
                        Fact('5 min', load.five.toStringAsFixed(2)),
                        Fact('15 min', load.fifteen.toStringAsFixed(2)),
                      ], minWidth: 80),
                    _Cores(cores: status.cpu.cores.toList()),
                  ],
                ),
              ),
              AccountPanel(
                icon: PhosphorIconsRegular.stack,
                title: 'Memory',
                detail: memory.cgroupLimitBytes != null
                    ? 'Container limit ${size(memory.cgroupLimitBytes)} (host ${size(memory.totalBytes)})'
                    : '${size(memory.totalBytes)} installed',
                trailing: _Big(percent(ratio(memory.usedBytes, limit))),
                child: AdminStack(
                  children: [
                    Sparkline(
                      values: [
                        for (final h in history) h.memoryUsedBytes.toDouble(),
                      ],
                      max: limit.toDouble(),
                      color: NfColors.focus,
                      height: 54,
                      label: 'Memory used',
                    ),
                    _BarRow(
                      'RAM',
                      Meter(
                        pct: ratio(memory.usedBytes, limit),
                        warn: 85,
                        bad: 95,
                        label: 'RAM used',
                      ),
                      '${size(memory.usedBytes)} / ${size(limit)}',
                    ),
                    _BarRow(
                      'Swap',
                      Meter(
                        pct: ratio(memory.swapUsedBytes, memory.swapTotalBytes),
                        warn: 50,
                        bad: 80,
                        label: 'Swap used',
                      ),
                      memory.swapTotalBytes > 0
                          ? '${size(memory.swapUsedBytes)} / ${size(memory.swapTotalBytes)}'
                          : 'none',
                    ),
                    Facts([
                      Fact('Available', size(memory.availableBytes)),
                      Fact('NoFlex resident', size(process.rssBytes)),
                    ]),
                  ],
                ),
              ),
              AccountPanel(
                icon: PhosphorIconsRegular.hardDrives,
                title: 'NoFlex process',
                detail:
                    'pid ${process.pid} · up ${uptime(status.server.uptimeSeconds)} · started ${relative(status.server.startedAt)}',
                trailing: _Big(percent(process.cpuPercent)),
                child: AdminStack(
                  children: [
                    Sparkline(
                      values: [for (final h in history) h.processCpuPercent],
                      color: NfColors.warning,
                      height: 54,
                      label: 'Server process CPU',
                    ),
                    Facts([
                      Fact(
                        'CPU of host',
                        percent(process.cpuPercentOfHost, digits: 1),
                      ),
                      Fact('Resident', size(process.rssBytes)),
                      Fact('Virtual', size(process.virtualBytes)),
                      Fact('Threads', '${process.threads ?? '—'}'),
                      Fact(
                        'Open files',
                        '${process.openFiles ?? '—'}${process.openFilesLimit != null ? ' / ${process.openFilesLimit}' : ''}',
                      ),
                      Fact('Started', stamp(status.server.startedAt)),
                    ]),
                  ],
                ),
              ),
              AccountPanel(
                icon: PhosphorIconsRegular.broadcast,
                title: 'Network',
                detail: 'Non-loopback interfaces combined.',
                trailing: Text(
                  '↓ ${rate(status.networkTotal.rxBytesPerSec)}\n↑ ${rate(status.networkTotal.txBytesPerSec)}',
                  textAlign: TextAlign.right,
                  style: nfText(13, weight: 700, features: NfType.tabular),
                ),
                child: AdminStack(
                  children: [
                    Sparkline(
                      values: [for (final h in history) h.rxBytesPerSec],
                      alt: [for (final h in history) h.txBytesPerSec],
                      color: NfColors.success,
                      height: 54,
                      label: 'Network receive (solid) and transmit (dashed)',
                    ),
                    Text(
                      'Solid is receive; dashed is transmit.',
                      style: NfType.caption,
                    ),
                  ],
                ),
              ),
            ],
          ),
          AccountPanel(
            icon: PhosphorIconsRegular.broadcast,
            title: 'Interfaces',
            detail: 'Rates over the last sample interval; totals since the interface came up.',
            child: AdminStack(
              children: [
                AdminCheck(
                  label:
                      'Show all${hidden > 0 && !showAll ? ' (+$hidden)' : ''}',
                  value: showAll,
                  onChanged: (value) => _showAllNics.value = value,
                ),
                AdminTable(
                  minWidth: 760,
                  empty: 'No active interfaces.',
                  columns: const [
                    AdminCol('Interface'),
                    AdminCol('State'),
                    AdminCol('Addresses', flex: true),
                    AdminCol('↓ Receive', numeric: true),
                    AdminCol('↑ Transmit', numeric: true),
                    AdminCol('Total ↓ / ↑', numeric: true),
                    AdminCol('Errors', numeric: true),
                  ],
                  rows: [
                    for (final nic in nics)
                      [
                        cellMono(
                          nic.loopback ? '${nic.name} (loopback)' : nic.name,
                        ),
                        StatePill(
                          nic.state,
                          tone: nic.state == 'up'
                              ? StateTone.ok
                              : StateTone.none,
                        ),
                        cellMono(_addresses(nic.addresses.toList())),
                        cellText(rate(nic.rxBytesPerSec)),
                        cellText(rate(nic.txBytesPerSec)),
                        cellText(
                          '${size(nic.rxTotalBytes)} / ${size(nic.txTotalBytes)}',
                        ),
                        cellText(
                          '${nic.rxErrorsTotal + nic.txErrorsTotal}',
                          color: nic.rxErrorsTotal + nic.txErrorsTotal > 0
                              ? NfColors.warning
                              : null,
                        ),
                      ],
                  ],
                ),
              ],
            ),
          ),
          AccountPanel(
            icon: PhosphorIconsRegular.hardDrives,
            title: 'Disks & folders',
            detail: 'Each folder NoFlex depends on and the filesystem that holds it. Cache sizes refresh every minute.',
            child: AdminTable(
              minWidth: 820,
              columns: const [
                AdminCol('Folder'),
                AdminCol('Path', flex: true),
                AdminCol('Filesystem'),
                AdminCol('Used'),
                AdminCol('Free', numeric: true),
                AdminCol('Contents', numeric: true),
              ],
              rows: [
                for (final location in status.storage)
                  [
                    cellText(
                      '${_storageKinds[location.kind.name] ?? location.kind.name}${location.libraryName == null ? '' : ' · ${location.libraryName}'}',
                      strong: true,
                    ),
                    cellMono(location.path),
                    location.error != null
                        ? Tooltip(
                            message: location.error!,
                            child: StatePill(
                              location.exists
                                  ? 'Error'
                                  : location.kind ==
                                        GStorageLocationKind.LIBRARY_ROOT
                                  ? 'Missing'
                                  : 'Not created',
                              tone:
                                  location.exists ||
                                      location.kind ==
                                          GStorageLocationKind.LIBRARY_ROOT
                                  ? StateTone.bad
                                  : StateTone.none,
                            ),
                          )
                        : cellMono(
                            '${location.fileSystem ?? '?'} ${location.mountPoint ?? ''}',
                          ),
                    (location.totalBytes ?? 0) > 0
                        ? SizedBox(
                            width: 130,
                            child: Row(
                              children: [
                                Expanded(
                                  child: Meter(
                                    pct: ratio(
                                      location.usedBytes,
                                      location.totalBytes,
                                    ),
                                    thin: true,
                                    label: '${location.path} used',
                                  ),
                                ),
                                const SizedBox(width: 8),
                                cellText(
                                  percent(
                                    ratio(
                                      location.usedBytes,
                                      location.totalBytes,
                                    ),
                                  ),
                                ),
                              ],
                            ),
                          )
                        : cellText('—', color: NfColors.text3),
                    cellText(
                      (location.totalBytes ?? 0) > 0
                          ? '${size(location.availableBytes)} of ${size(location.totalBytes)}'
                          : '—',
                    ),
                    cellText(
                      location.contentBytes == null
                          ? '—'
                          : size(location.contentBytes),
                    ),
                  ],
              ],
            ),
          ),
        ],
      );
    },
  );

  String _addresses(List<String> addresses) {
    if (addresses.isEmpty) return '—';
    final shown = addresses.take(2).join(', ');
    return addresses.length > 2 ? '$shown +${addresses.length - 2}' : shown;
  }
}

class _Big extends StatelessWidget {
  const _Big(this.value);
  final String value;

  @override
  Widget build(BuildContext context) => Text(
    value,
    style: nfText(
      22,
      weight: 700,
      letterSpacing: -0.5,
      features: NfType.tabular,
    ),
  );
}

class _BarRow extends StatelessWidget {
  const _BarRow(this.label, this.meter, this.value);
  final String label;
  final Widget meter;
  final String value;

  @override
  Widget build(BuildContext context) => Row(
    children: [
      SizedBox(
        width: 44,
        child: Text(label, style: nfText(13, color: NfColors.text2)),
      ),
      Expanded(child: meter),
      const SizedBox(width: 12),
      Text(value, style: NfType.caption),
    ],
  );
}

/// Even rows: up to 16 bars per row, so 32 threads read as two clean rows.
class _Cores extends StatelessWidget {
  const _Cores({required this.cores});
  final List<GSystemStatusFields_cpu_cores> cores;

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final perRow = cores.length.clamp(1, 16);
      const gap = 3.0;
      final width = (constraints.maxWidth - gap * (perRow - 1)) / perRow;
      return Wrap(
        spacing: gap,
        runSpacing: 4,
        children: [
          for (final core in cores)
            Tooltip(
              message:
                  'Thread ${core.index}: ${percent(core.usagePercent)} at ${core.frequencyMhz} MHz',
              child: SizedBox(
                width: width,
                child: Column(
                  children: [
                    Container(
                      height: 44,
                      alignment: Alignment.bottomCenter,
                      decoration: BoxDecoration(
                        color: NfColors.surface2,
                        borderRadius: BorderRadius.circular(4),
                      ),
                      child: FractionallySizedBox(
                        heightFactor: core.usagePercent.clamp(2, 100) / 100,
                        widthFactor: 1,
                        child: DecoratedBox(
                          decoration: BoxDecoration(
                            color: core.usagePercent >= 85
                                ? NfColors.warning
                                : NfColors.accent,
                            borderRadius: BorderRadius.circular(3),
                          ),
                        ),
                      ),
                    ),
                    const SizedBox(height: 3),
                    Text(
                      '${core.index}',
                      style: nfText(9, color: NfColors.text3),
                    ),
                  ],
                ),
              ),
            ),
        ],
      );
    },
  );
}
