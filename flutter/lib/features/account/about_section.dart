// SPDX-License-Identifier: AGPL-3.0-only
//
// What this app is, where its source is, and the licenses of what it's built
// from, which the LGPL and permissive licenses alike ask it to show.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:package_info_plus/package_info_plus.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../browse/work_browser.dart';
import '../../layout/window.dart';
import '../../services/device_identity.dart';
import '../../services/licenses.dart';
import '../../services/nav_service.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_state.dart';
import 'account_widgets.dart';

const sourceUrl = 'https://github.com/NoFlexMedia/noflex';

class AboutSection extends StatefulWidget {
  const AboutSection({super.key});

  @override
  State<AboutSection> createState() => _AboutSectionState();
}

class _AboutSectionState extends State<AboutSection> {
  final _version = signal<String?>(null);

  @override
  void initState() {
    super.initState();
    unawaited(
      PackageInfo.fromPlatform().then(
        (info) => _version.value = '${info.version} (${info.buildNumber})',
      ),
    );
  }

  @override
  Widget build(BuildContext context) => AccountColumn(
    children: [
      SignalBuilder(
        builder: (context) => AccountPanel(
          icon: PhosphorIconsRegular.info,
          title: getIt<DeviceIdentity>().clientName,
          detail: switch (_version.value) {
            final version? => 'Version $version',
            null => null,
          },
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'NoFlex is free software: you can share and change it under '
                'the GNU Affero General Public License, version 3. Its source '
                'code is at',
                style: NfType.bodyMuted,
              ),
              const SizedBox(height: 4),
              SelectableText(sourceUrl, style: nfText(15, weight: 550)),
            ],
          ),
        ),
      ),
      AccountPanel(
        icon: PhosphorIconsRegular.scales,
        title: 'Open source licenses',
        detail: 'The software and fonts NoFlex is built with',
        child: NfButton(
          label: 'Show licenses',
          expand: true,
          onPressed: () =>
              unawaited(getIt<NavService>().push(const LicensesScreen())),
        ),
      ),
    ],
  );
}

class LicensesScreen extends StatefulWidget {
  const LicensesScreen({super.key});

  @override
  State<LicensesScreen> createState() => _LicensesScreenState();
}

class _LicensesScreenState extends State<LicensesScreen> {
  final _packages = signal<List<(String, List<LicenseEntry>)>?>(null);
  final _failed = signal<Object?>(null);

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  Future<void> _load() async {
    _failed.value = null;
    try {
      _packages.value = await licensesByPackage();
    } catch (error) {
      _failed.value = error;
    }
  }

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return Scaffold(
      appBar: const NfPageBar(title: 'Open source licenses'),
      body: SignalBuilder(
        builder: (context) {
          if (_failed.value case final error?) {
            return NfState.error(
              title: 'The licenses didn’t load',
              message: '$error',
              retry: () => unawaited(_load()),
            );
          }
          final packages = _packages.value;
          if (packages == null) {
            return Padding(
              padding: EdgeInsets.fromLTRB(gutter, 8, gutter, 0),
              child: const NfSkeletonRows(count: 8),
            );
          }
          return ListView.builder(
            padding: EdgeInsets.fromLTRB(gutter - 4, 4, gutter - 4, 32),
            itemCount: packages.length,
            itemBuilder: (context, index) {
              final (name, entries) = packages[index];
              return _PackageRow(name: name, entries: entries);
            },
          );
        },
      ),
    );
  }
}

class _PackageRow extends StatelessWidget {
  const _PackageRow({required this.name, required this.entries});

  final String name;
  final List<LicenseEntry> entries;

  @override
  Widget build(BuildContext context) => InkWell(
    onTap: () => unawaited(
      getIt<NavService>().push(_LicenseScreen(name: name, entries: entries)),
    ),
    borderRadius: BorderRadius.circular(NfRadii.s),
    child: Padding(
      padding: const EdgeInsets.symmetric(vertical: 12, horizontal: 4),
      child: Row(
        children: [
          Expanded(child: Text(name, style: nfText(15, weight: 550))),
          if (entries.length > 1)
            Text('${entries.length} licenses', style: NfType.small),
          const SizedBox(width: 8),
          const Icon(
            PhosphorIconsRegular.caretRight,
            size: 16,
            color: NfColors.text3,
          ),
        ],
      ),
    ),
  );
}

class _LicenseScreen extends StatelessWidget {
  const _LicenseScreen({required this.name, required this.entries});

  final String name;
  final List<LicenseEntry> entries;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    final paragraphs = [
      for (final (index, entry) in entries.indexed) ...[
        if (index > 0) null,
        ...entry.paragraphs,
      ],
    ];
    return Scaffold(
      appBar: NfPageBar(title: name),
      body: SelectionArea(
        child: ListView.builder(
          padding: EdgeInsets.fromLTRB(gutter, 8, gutter, 32),
          itemCount: paragraphs.length,
          itemBuilder: (context, index) => switch (paragraphs[index]) {
            null => const Padding(
              padding: EdgeInsets.symmetric(vertical: 16),
              child: Divider(color: NfColors.line, height: 1),
            ),
            final paragraph => Padding(
              padding: EdgeInsets.only(
                bottom: 10,
                left: paragraph.indent > 0 ? paragraph.indent * 16.0 : 0,
              ),
              child: Text(
                paragraph.text,
                textAlign: paragraph.indent == LicenseParagraph.centeredIndent
                    ? TextAlign.center
                    : TextAlign.start,
                style: NfType.small,
              ),
            ),
          },
        ),
      ),
    );
  }
}
