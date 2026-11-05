// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../layout/window.dart';
import '../../theme.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_state.dart';
import '../player/playback_service.dart';
import 'channel_logo.dart';
import 'guide_math.dart';
import 'live_tv_controller.dart';

/// Past this many, finding one by eye stops being reasonable.
const _filterAbove = 12;

class ChannelsView extends StatefulWidget {
  const ChannelsView({super.key, required this.controller});
  final LiveTvController controller;

  @override
  State<ChannelsView> createState() => _ChannelsViewState();
}

class _ChannelsViewState extends State<ChannelsView> {
  final _field = TextEditingController();
  final _filter = signal('');

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    return SignalBuilder(
      builder: (context) {
        final channels = widget.controller.visibleChannels.value;
        final query = _filter.value.trim().toLowerCase();
        final visible = query.isEmpty
            ? channels
            : channels
                  .where(
                    (c) => '${c.number ?? ''} ${c.name} ${c.group ?? ''}'
                        .toLowerCase()
                        .contains(query),
                  )
                  .toList();
        return CustomScrollView(
          slivers: [
            if (channels.length > _filterAbove)
              SliverPadding(
                padding: EdgeInsets.fromLTRB(gutter, 6, gutter, 12),
                sliver: SliverToBoxAdapter(
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 360),
                      child: NfField(
                        controller: _field,
                        hint: 'Filter by name, number or group',
                        autocorrect: false,
                        prefixIcon: const Icon(
                          PhosphorIconsRegular.magnifyingGlass,
                          size: 16,
                          color: NfColors.text3,
                        ),
                        onChanged: (value) => _filter.value = value,
                      ),
                    ),
                  ),
                ),
              ),
            if (visible.isEmpty)
              SliverFillRemaining(
                hasScrollBody: false,
                child: NfState(
                  title: channels.isEmpty
                      ? 'No channels right now'
                      : 'No channels match “${_filter.value.trim()}”',
                  message: channels.isEmpty
                      ? 'Your scheduled recordings are still on the DVR tab.'
                      : null,
                ),
              )
            else
              SliverPadding(
                padding: EdgeInsets.fromLTRB(gutter, 6, gutter, 32),
                sliver: SliverGrid.builder(
                  gridDelegate: SliverGridDelegateWithMaxCrossAxisExtent(
                    maxCrossAxisExtent: 360,
                    crossAxisSpacing: 12,
                    mainAxisSpacing: 12,
                    mainAxisExtent: MediaQuery.textScalerOf(context).scale(142),
                  ),
                  itemCount: visible.length,
                  itemBuilder: (_, index) => _ChannelCard(
                    controller: widget.controller,
                    channel: visible[index],
                  ),
                ),
              ),
          ],
        );
      },
    );
  }
}

class _ChannelCard extends SignalWidget {
  const _ChannelCard({required this.controller, required this.channel});
  final LiveTvController controller;
  final LiveChannel channel;

  @override
  Widget build(BuildContext context) {
    final playback = getIt<PlaybackService>();
    final now = controller.now.value;
    final (:current, next: _) = nowAndNext(
      controller.programsByChannel.value[channel.id] ?? const [],
      now,
    );
    final starting = playback.preparing.value == channel.id;
    return Semantics(
      button: true,
      label: [
        'Watch ${channel.name}',
        if (current != null) 'now showing ${current.title}',
      ].join(', '),
      excludeSemantics: true,
      child: NfPanel(
        padding: const EdgeInsets.all(14),
        onTap: starting
            ? null
            : () =>
                  playback.playLive(channelId: channel.id, name: channel.name),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                ChannelLogo(
                  name: channel.name,
                  logoUrl: channel.logoUrl,
                  size: 46,
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      if (channel.numberLabel case final number?)
                        Text(
                          number,
                          style: nfText(
                            11.5,
                            weight: 700,
                            color: NfColors.accent,
                            features: NfType.tabular,
                          ),
                        ),
                      Text(
                        channel.name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: nfText(14.5, weight: 650),
                      ),
                      Text(
                        channel.groupLabel ?? 'Live channel',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: NfType.caption,
                      ),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                Container(
                  width: 34,
                  height: 34,
                  decoration: const BoxDecoration(
                    color: NfColors.surface3,
                    shape: BoxShape.circle,
                  ),
                  child: starting
                      ? const Padding(
                          padding: EdgeInsets.all(9),
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Icon(
                          PhosphorIconsFill.play,
                          size: 15,
                          color: NfColors.text,
                        ),
                ),
              ],
            ),
            const Spacer(),
            const Divider(height: 1),
            const SizedBox(height: 10),
            if (current == null)
              Text('No guide information', style: NfType.caption)
            else ...[
              Row(
                children: [
                  Text(
                    'NOW',
                    style: nfText(
                      10.5,
                      weight: 700,
                      color: NfColors.danger,
                      letterSpacing: 1.2,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      current.title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: nfText(13.5, weight: 550, color: NfColors.text2),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 6),
              NfProgressBar(value: current.progress(now), height: 3),
            ],
          ],
        ),
      ),
    );
  }
}
