// SPDX-License-Identifier: AGPL-3.0-only
//
// Guide, channels and DVR. Tuner sources are managed on the web, so a server
// without any gets a page explaining that rather than three empty tabs.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:intl/intl.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../layout/window.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../services/tokens.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../browse/work_browser.dart';
import '../player/playback_service.dart';
import 'channels_view.dart';
import 'dvr_view.dart';
import 'guide_view.dart';
import 'live_tv_controller.dart';

enum _Tab { guide, channels, dvr }

class LiveTvScreen extends StatefulWidget {
  const LiveTvScreen({super.key});

  @override
  State<LiveTvScreen> createState() => _LiveTvScreenState();
}

class _LiveTvScreenState extends State<LiveTvScreen> {
  late final _controller = LiveTvController(shouldPoll: _isVisible);
  final _tab = signal(_Tab.guide);

  bool _isVisible() {
    if (!mounted || ModalRoute.of(context)?.isCurrent != true) return false;
    if (getIt<PlaybackService>().current.value != null) return false;
    final state = WidgetsBinding.instance.lifecycleState;
    return state != AppLifecycleState.paused &&
        state != AppLifecycleState.hidden;
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: NfPageBar(
      title: 'Live TV',
      actions: [_Clock(controller: _controller)],
    ),
    body: SignalBuilder(
      builder: (context) {
        final live = _controller.live;
        if (live.waiting.value) {
          return const Center(child: CircularProgressIndicator());
        }
        if (live.data.value == null) {
          return NfState.error(
            title: 'Live TV is off the air',
            message: live.error.value?.toString(),
            retry: live.refetch,
          );
        }
        if (_controller.isEmpty.value) {
          return _NoSources(onCheckAgain: live.refetch);
        }
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _TabRow(controller: _controller, tab: _tab),
            Expanded(
              child: IndexedStack(
                index: _tab.value.index,
                sizing: StackFit.expand,
                children: [
                  GuideView(controller: _controller),
                  ChannelsView(controller: _controller),
                  DvrView(controller: _controller),
                ],
              ),
            ),
          ],
        );
      },
    ),
  );
}

class _Clock extends SignalWidget {
  const _Clock({required this.controller});
  final LiveTvController controller;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(right: 12),
    child: Center(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
        decoration: BoxDecoration(
          color: NfColors.surface,
          borderRadius: BorderRadius.circular(NfRadii.pill),
          border: Border.all(color: NfColors.line),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Icon(
              PhosphorIconsFill.circle,
              size: 8,
              color: NfColors.danger,
            ),
            const SizedBox(width: 8),
            Text(
              DateFormat.jms().format(controller.clock.value),
              style: nfText(13, weight: 650, features: NfType.tabular),
            ),
          ],
        ),
      ),
    ),
  );
}

class _TabRow extends SignalWidget {
  const _TabRow({required this.controller, required this.tab});
  final LiveTvController controller;
  final Signal<_Tab> tab;

  Future<void> _chooseGroup() async {
    const all = '';
    final choice = await pickOption<String>(
      title: 'Channel group',
      selected: controller.group.value ?? all,
      options: [
        const NfOption(all, 'All channels'),
        for (final name in controller.groups.value) NfOption(name, name),
      ],
    );
    if (choice != null) controller.group.value = choice.isEmpty ? null : choice;
  }

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    final channelCount = controller.channels.value.length;
    final upcoming = controller.schedules.value
        .where((s) => s.status == GRecordingScheduleStatus.SCHEDULED)
        .length;
    final recordingNow = controller.recordings.value
        .where((r) => r.status == GRecordingStatus.RUNNING)
        .length;
    final showGroups =
        controller.groups.value.length > 1 && tab.value != _Tab.dvr;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: EdgeInsets.fromLTRB(gutter, 0, gutter, 6),
          child: Text(
            [
              '$channelCount ${channelCount == 1 ? 'channel' : 'channels'}',
              if (recordingNow > 0) '$recordingNow recording now',
            ].join(' · '),
            style: NfType.small,
          ),
        ),
        SizedBox(
          height: 52,
          child: ListView(
            scrollDirection: Axis.horizontal,
            padding: EdgeInsets.fromLTRB(gutter, 6, gutter, 8),
            children: [
              for (final (value, label, count) in [
                (_Tab.guide, 'Guide', 0),
                (_Tab.channels, 'Channels', channelCount),
                (_Tab.dvr, 'DVR', upcoming),
              ])
                Padding(
                  padding: const EdgeInsets.only(right: 8),
                  child: NfChoiceChip(
                    label: label,
                    detail: count > 0 ? '$count' : null,
                    selected: tab.value == value,
                    onTap: () => tab.value = value,
                  ),
                ),
              if (showGroups)
                NfChoiceChip(
                  label: controller.group.value ?? 'All channels',
                  selected: false,
                  leading: const Icon(PhosphorIconsRegular.listBullets),
                  onTap: _chooseGroup,
                ),
            ],
          ),
        ),
      ],
    );
  }
}

/// Live TV is optional and off by default, so this is what most servers show.
/// It's the only place an unset-up feature can explain itself, and an
/// administrator has nowhere in this app to fix it, so it points at the web.
class _NoSources extends StatelessWidget {
  const _NoSources({required this.onCheckAgain});
  final VoidCallback onCheckAgain;

  Future<void> _copySourcesLink() async {
    final link = getIt<TokenStore>().absolute('/live-tv?tab=sources');
    await Clipboard.setData(ClipboardData(text: link));
    getIt<ToastService>().show(
      'Link to Live TV sources copied',
      kind: ToastKind.success,
    );
  }

  @override
  Widget build(BuildContext context) {
    final isAdmin = getIt<SessionService>().viewer.value?.isAdmin ?? false;
    return LayoutBuilder(
      builder: (context, constraints) => SingleChildScrollView(
        child: ConstrainedBox(
          constraints: BoxConstraints(minHeight: constraints.maxHeight),
          child: Center(
            child: NfState(
              icon: PhosphorIconsRegular.television,
              title: 'No live TV on this server yet',
              message: isAdmin
                  ? 'Add an M3U tuner source, with an XMLTV guide if you have one. Its channels show up here on their own. Sources are managed from NoFlex on the web.'
                  : 'Nobody has connected a tuner or IPTV source yet. Ask your server’s administrator to add one.',
              child: Padding(
                padding: const EdgeInsets.only(top: 22),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    const _Promise(
                      icon: PhosphorIconsRegular.clock,
                      title: 'Guide',
                      detail: 'What’s on now and next, across every channel.',
                    ),
                    const _Promise(
                      icon: PhosphorIconsRegular.playCircle,
                      title: 'Channels',
                      detail: 'Tap one and it plays.',
                    ),
                    const _Promise(
                      icon: PhosphorIconsRegular.record,
                      title: 'DVR',
                      detail: 'Schedule a recording from any program.',
                    ),
                    const SizedBox(height: 16),
                    Wrap(
                      alignment: WrapAlignment.center,
                      spacing: 10,
                      runSpacing: 10,
                      children: [
                        if (isAdmin)
                          NfButton(
                            label: 'Copy link to sources',
                            style: NfButtonStyle.primary,
                            icon: const Icon(PhosphorIconsRegular.link),
                            onPressed: () => unawaited(_copySourcesLink()),
                          ),
                        NfButton(label: 'Check again', onPressed: onCheckAgain),
                      ],
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _Promise extends StatelessWidget {
  const _Promise({
    required this.icon,
    required this.title,
    required this.detail,
  });

  final IconData icon;
  final String title;
  final String detail;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 6),
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Icon(icon, size: 18, color: NfColors.text3),
        const SizedBox(width: 12),
        Expanded(
          child: Text.rich(
            TextSpan(
              children: [
                TextSpan(text: '$title  ', style: nfText(13.5, weight: 600)),
                TextSpan(text: detail, style: NfType.small),
              ],
            ),
          ),
        ),
      ],
    ),
  );
}
