// SPDX-License-Identifier: AGPL-3.0-only
//
// Titles marked to watch next, which the server prepares ahead of anything it
// guesses at. Marking happens from a title's page.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/fragments/__generated__/smart_mark_fields.data.gql.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_state.dart';
import 'account_controller.dart';
import 'account_widgets.dart';

class SmartMarksSection extends StatefulWidget {
  const SmartMarksSection({super.key, required this.controller});
  final AccountController controller;

  @override
  State<SmartMarksSection> createState() => _SmartMarksSectionState();
}

class _SmartMarksSectionState extends State<SmartMarksSection> {
  @override
  void initState() {
    super.initState();
    unawaited(widget.controller.loadSmartMarks());
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final marks = widget.controller.smartMarks.value;
      final prepared = widget.controller.preparedVersions.value;
      if (marks.isEmpty && prepared.isEmpty) {
        return const AccountColumn(
          children: [
            NfState(
              icon: PhosphorIconsRegular.sparkle,
              title: 'Nothing marked yet',
              message: 'Open a film or show, choose ••• → Mark for smart transcoding, and it’ll be ready before you sit down to watch.',
            ),
          ],
        );
      }
      return AccountColumn(
        children: [
          if (marks.isNotEmpty)
            AccountPanel(
              icon: PhosphorIconsRegular.sparkle,
              title: 'Marked to watch next',
              detail: 'Prepared in this order, ahead of anything the server guesses at. Each stays until you watch it or unmark it.',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final mark in marks)
                    _MarkRow(
                      mark: mark,
                      onUnmark: () => unawaited(widget.controller.unmark(mark)),
                    ),
                ],
              ),
            ),
          if (prepared.isNotEmpty)
            AccountPanel(
              icon: PhosphorIconsRegular.checkCircle,
              title: 'Ready to play',
              detail: 'Already prepared on this server, and used by any device it suits. Unmarking a title doesn’t remove it. The least recently played make room when space runs out.',
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final version in prepared)
                    AccountRow(
                      icon: PhosphorIconsRegular.checkCircle,
                      title: version.work?.title ?? 'A removed title',
                      detail: [
                        version.description,
                        bytes(version.sizeBytes),
                        if (version.lastUsedAt case final played?)
                          'last played ${ago(played)}'
                        else
                          'not played yet',
                      ].join(' · '),
                    ),
                ],
              ),
            ),
        ],
      );
    },
  );
}

(String, String?) _describe(GSmartMarkFields mark) => switch (mark.state) {
  GSmartTranscodeState.WAITING => (
    'In the queue',
    'It’s next, once nobody is watching anything.',
  ),
  GSmartTranscodeState.PREPARING => (
    'Preparing ${(mark.progress * 100).round()}%',
    null,
  ),
  GSmartTranscodeState.READY => ('Ready', null),
  GSmartTranscodeState.NOT_NEEDED => (
    'Plays as it is',
    'Nothing to prepare: the original already plays there.',
  ),
  GSmartTranscodeState.TOO_LARGE => (
    'Too large',
    'It’s bigger than this server allows a prepared version to be.',
  ),
  GSmartTranscodeState.NO_DEVICES => (
    'No devices',
    'None of your devices has smart transcoding on. Turn it on under Devices.',
  ),
  GSmartTranscodeState.NO_SPACE => (
    'Waiting for space',
    'Other marked titles are using all the space this server sets aside.',
  ),
  GSmartTranscodeState.FAILED => (
    'Failed',
    'The server couldn’t prepare this. Unmark it and mark it again to retry.',
  ),
  GSmartTranscodeState.DISABLED => (
    'Off on this server',
    'Smart transcoding is off in this server’s configuration.',
  ),
  _ => (mark.state.name, null),
};

class _MarkRow extends StatelessWidget {
  const _MarkRow({required this.mark, required this.onUnmark});
  final GSmartMarkFields mark;
  final VoidCallback onUnmark;

  @override
  Widget build(BuildContext context) {
    final (label, detail) = _describe(mark);
    final scope = mark.device == null
        ? 'For your devices with smart transcoding on'
        : 'For ${mark.device!.name} only';
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        AccountRow(
          icon: PhosphorIconsRegular.sparkle,
          title: mark.work.title,
          detail: [label, scope, ?detail].join(' · '),
          trailing: NfButton(
            label: 'Unmark',
            style: NfButtonStyle.ghost,
            size: NfButtonSize.small,
            onPressed: onUnmark,
          ),
        ),
        if (mark.state == GSmartTranscodeState.PREPARING)
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: LinearProgressIndicator(
              value: mark.progress.clamp(0.0, 1.0),
              color: NfColors.accent,
              backgroundColor: NfColors.surface2,
            ),
          ),
      ],
    );
  }
}
