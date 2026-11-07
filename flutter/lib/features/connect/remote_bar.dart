// SPDX-License-Identifier: AGPL-3.0-only

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../models/work.dart';
import '../../theme.dart';
import '../../utils/device_icon.dart';
import '../../utils/format.dart';
import '../../widgets/artwork.dart';
import '../../widgets/nf_button.dart';
import 'connect_service.dart';

const _skipBack = 10000;
const _skipForward = 30000;

/// What another of the viewer's devices is playing, steered from here the way
/// the player on this one is: play and pause, skip, scrub, stop.
class RemoteBar extends StatefulWidget {
  const RemoteBar({super.key});

  @override
  State<RemoteBar> createState() => _RemoteBarState();
}

class _RemoteBarState extends State<RemoteBar> {
  // Where a drag of the scrubber is, until it lets go.
  final _scrub = signal<double?>(null);

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final connect = getIt<ConnectService>();
      final device = connect.active.value;
      final playback = device?.playback;
      if (device == null || playback == null) return const SizedBox.shrink();
      final card = playback.work;
      final work = card == null ? null : WorkSummary.fromCard(card);
      final length = playback.durationMs ?? 0;
      final at = connect.position(device);
      final shown = _scrub.value ?? at.toDouble();
      final compact = MediaQuery.sizeOf(context).width < 560;
      void seek(int to) => connect.control(
        device,
        GDeviceControl.SEEK,
        positionMs: length > 0
            ? to.clamp(0, length)
            : to < 0
            ? 0
            : to,
      );
      return Semantics(
        container: true,
        label: 'Playing on ${device.name}',
        child: Container(
          margin: const EdgeInsets.fromLTRB(8, 0, 8, 8),
          padding: const EdgeInsets.fromLTRB(8, 8, 8, 4),
          decoration: BoxDecoration(
            color: const Color(0xF0080A0C),
            borderRadius: BorderRadius.circular(NfRadii.l),
            border: Border.all(color: NfColors.lineStrong),
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Row(
                children: [
                  if (work != null)
                    ClipRRect(
                      borderRadius: BorderRadius.circular(NfRadii.xs),
                      child: SizedBox(
                        width: 40,
                        height: 60,
                        child: Artwork(work: work),
                      ),
                    ),
                  const SizedBox(width: 12),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          work?.title ?? 'Playing',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: NfType.body,
                        ),
                        Row(
                          children: [
                            Icon(
                              deviceIconFor(device.name, device.client),
                              size: 13,
                              color: NfColors.text2,
                            ),
                            const SizedBox(width: 6),
                            Expanded(
                              child: Text(
                                [
                                  device.name,
                                  if (work != null && work.subtitle.isNotEmpty)
                                    work.subtitle,
                                ].join(' · '),
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: NfType.caption,
                              ),
                            ),
                          ],
                        ),
                      ],
                    ),
                  ),
                  if (!compact)
                    NfIconButton(
                      icon: PhosphorIconsRegular.arrowCounterClockwise,
                      label: 'Back 10 seconds',
                      size: 40,
                      onPressed: () => seek(at - _skipBack),
                    ),
                  if (device.buffering)
                    // Loading: the spinner stands where the play button does, and still pauses.
                    Semantics(
                      button: true,
                      label: 'Loading',
                      child: InkResponse(
                        onTap: () =>
                            connect.control(device, GDeviceControl.PAUSE),
                        child: const SizedBox.square(
                          dimension: 44,
                          child: Center(
                            child: SizedBox.square(
                              dimension: 22,
                              child: CircularProgressIndicator(
                                strokeWidth: 2,
                                color: NfColors.text,
                              ),
                            ),
                          ),
                        ),
                      ),
                    )
                  else
                    NfIconButton(
                      icon: device.playing
                          ? PhosphorIconsFill.pause
                          : PhosphorIconsFill.play,
                      label: device.playing ? 'Pause' : 'Play',
                      size: 44,
                      background: NfColors.surface2,
                      onPressed: () => connect.control(
                        device,
                        device.playing
                            ? GDeviceControl.PAUSE
                            : GDeviceControl.PLAY,
                      ),
                    ),
                  if (!compact)
                    NfIconButton(
                      icon: PhosphorIconsRegular.arrowClockwise,
                      label: 'Forward 30 seconds',
                      size: 40,
                      onPressed: () => seek(at + _skipForward),
                    ),
                  NfIconButton(
                    icon: PhosphorIconsFill.stop,
                    label: 'Stop playing on ${device.name}',
                    size: 40,
                    onPressed: () =>
                        connect.control(device, GDeviceControl.STOP),
                  ),
                ],
              ),
              SliderTheme(
                data: SliderTheme.of(context).copyWith(
                  trackHeight: 3,
                  thumbShape: const RoundSliderThumbShape(
                    enabledThumbRadius: 6,
                  ),
                  overlayShape: SliderComponentShape.noOverlay,
                  activeTrackColor: NfColors.accent,
                  inactiveTrackColor: NfColors.surface3,
                  thumbColor: NfColors.text,
                ),
                child: Row(
                  children: [
                    Text(timecode(shown.round()), style: NfType.caption),
                    Expanded(
                      child: Slider(
                        value: length > 0
                            ? shown.clamp(0, length.toDouble())
                            : 0,
                        max: length > 0 ? length.toDouble() : 1,
                        onChanged: length > 0 ? (v) => _scrub.value = v : null,
                        onChangeEnd: (v) {
                          _scrub.value = null;
                          seek(v.round());
                        },
                      ),
                    ),
                    Text(
                      length > 0 ? timecode(length) : '',
                      style: NfType.caption,
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      );
    },
  );
}
