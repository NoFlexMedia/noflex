// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../services/session.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../utils/tracks.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../player/decoder_probe.dart';
import '../player/decoders.dart';
import '../player/playback_service.dart';
import 'account_controller.dart';
import 'account_widgets.dart';

class ProfileSection extends StatelessWidget {
  const ProfileSection({super.key, required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) => AccountColumn(
    children: [
      _BrowsingPanel(controller: controller),
      _PlaybackPanel(controller: controller),
      _ThisDevicePanel(controller: controller),
      _PasswordPanel(controller: controller),
    ],
  );
}

class _BrowsingPanel extends SignalWidget {
  const _BrowsingPanel({required this.controller});
  final AccountController controller;

  @override
  Widget build(BuildContext context) => AccountPanel(
    icon: PhosphorIconsRegular.books,
    title: 'Browsing',
    detail: 'NoFlex browses by movies, shows and other content types across your libraries',
    child: AccountToggle(
      title: 'Show libraries in the sidebar',
      detail: 'List each library too, for browsing by where files live.',
      value: getIt<SessionService>().showLibraries.value,
      onChanged: (show) => unawaited(controller.setShowLibraries(show)),
    ),
  );
}

class _PasswordPanel extends StatefulWidget {
  const _PasswordPanel({required this.controller});
  final AccountController controller;

  @override
  State<_PasswordPanel> createState() => _PasswordPanelState();
}

class _PasswordPanelState extends State<_PasswordPanel> {
  final _current = TextEditingController();
  final _next = TextEditingController();
  final _confirm = TextEditingController();
  final _busy = signal(false);

  /// The fields' text as it changes, for what the button and hint depend on.
  final _text = signal((current: '', next: '', confirm: ''));

  void _changed(String _) => _text.value = (
    current: _current.text,
    next: _next.text,
    confirm: _confirm.text,
  );

  @override
  void dispose() {
    _current.dispose();
    _next.dispose();
    _confirm.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    _busy.value = true;
    final changed = await widget.controller.changePassword(
      _current.text,
      _next.text,
    );
    if (!mounted) return;
    _busy.value = false;
    if (changed) {
      _current.clear();
      _next.clear();
      _confirm.clear();
      _changed('');
    }
  }

  @override
  Widget build(BuildContext context) => AccountPanel(
    icon: PhosphorIconsRegular.lock,
    title: 'Password',
    detail: 'Changing it signs out every other device',
    child: AutofillGroup(
      child: SignalBuilder(
        builder: (context) {
          final text = _text.value;
          final mismatch = text.confirm.isNotEmpty && text.confirm != text.next;
          final ready =
              text.current.isNotEmpty &&
              text.next.length >= 8 &&
              text.confirm == text.next;
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              NfField(
                controller: _current,
                label: 'Current password',
                obscure: true,
                autocorrect: false,
                autofillHints: const [AutofillHints.password],
                textInputAction: TextInputAction.next,
                onChanged: _changed,
              ),
              const SizedBox(height: 12),
              NfField(
                controller: _next,
                label: 'New password',
                obscure: true,
                autocorrect: false,
                autofillHints: const [AutofillHints.newPassword],
                textInputAction: TextInputAction.next,
                onChanged: _changed,
              ),
              const SizedBox(height: 4),
              Text('At least 8 characters.', style: NfType.caption),
              const SizedBox(height: 12),
              NfField(
                controller: _confirm,
                label: 'Confirm new password',
                obscure: true,
                autocorrect: false,
                autofillHints: const [AutofillHints.newPassword],
                textInputAction: TextInputAction.done,
                onChanged: _changed,
                onSubmitted: (_) {
                  if (ready && !_busy.value) unawaited(_submit());
                },
              ),
              if (mismatch) ...[
                const SizedBox(height: 4),
                Text(
                  'Passwords don’t match yet.',
                  style: nfText(12, color: NfColors.danger),
                ),
              ],
              const SizedBox(height: 14),
              Align(
                alignment: Alignment.centerRight,
                child: NfButton(
                  label: 'Update password',
                  style: NfButtonStyle.primary,
                  size: NfButtonSize.small,
                  busy: _busy.value,
                  onPressed: ready ? () => unawaited(_submit()) : null,
                ),
              ),
            ],
          );
        },
      ),
    ),
  );
}

class _PlaybackPanel extends StatefulWidget {
  const _PlaybackPanel({required this.controller});
  final AccountController controller;

  @override
  State<_PlaybackPanel> createState() => _PlaybackPanelState();
}

class _PlaybackPanelState extends State<_PlaybackPanel> {
  final _audio = TextEditingController();
  final _subtitleLanguages = TextEditingController();
  final _subtitlesOn = signal(false);
  final _busy = signal(false);
  late final void Function() _stopSeeding;

  @override
  void initState() {
    super.initState();
    _stopSeeding = effect(() {
      widget.controller.storedPlaybackJson.value;
      untracked(_seed);
    });
  }

  /// What's stored, unless it isn't known yet, when the session's copy from
  /// signing in is as good as it gets.
  void _seed() {
    final prefs = widget.controller.storedPlaybackJson.value == null
        ? getIt<SessionService>().playbackPrefs.value
        : PlaybackPrefs.fromJson(widget.controller.storedPlayback.value);
    _audio.text = prefs.audioLanguages.join(', ');
    _subtitleLanguages.text = prefs.subtitleLanguages.join(', ');
    _subtitlesOn.value = prefs.subtitlesOn;
  }

  @override
  void dispose() {
    _stopSeeding();
    _audio.dispose();
    _subtitleLanguages.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    _busy.value = true;
    await widget.controller.savePlayback(
      PlaybackPrefs(
        audioLanguages: splitLanguages(_audio.text),
        subtitlesOn: _subtitlesOn.value,
        subtitleLanguages: splitLanguages(_subtitleLanguages.text),
      ),
    );
    if (mounted) _busy.value = false;
  }

  @override
  Widget build(BuildContext context) => AccountPanel(
    icon: PhosphorIconsRegular.translate,
    title: 'Playback preferences',
    detail: 'Synced to your account, so every NoFlex client can honor them',
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        NfField(
          controller: _audio,
          label: 'Preferred audio languages',
          hint: 'en, ja',
          autocorrect: false,
        ),
        const SizedBox(height: 4),
        Text('Comma-separated, most preferred first.', style: NfType.caption),
        const SizedBox(height: 12),
        SignalBuilder(
          builder: (context) => AccountToggle(
            title: 'Subtitles on by default',
            detail: 'Pick a matching track when one exists.',
            value: _subtitlesOn.value,
            onChanged: (value) => _subtitlesOn.value = value,
          ),
        ),
        const SizedBox(height: 8),
        NfField(
          controller: _subtitleLanguages,
          label: 'Preferred subtitle languages',
          hint: 'en',
          autocorrect: false,
        ),
        const SizedBox(height: 4),
        Text('Used when subtitles are on.', style: NfType.caption),
        const SizedBox(height: 14),
        Align(
          alignment: Alignment.centerRight,
          child: SignalBuilder(
            builder: (context) => NfButton(
              label: 'Save preferences',
              style: NfButtonStyle.primary,
              size: NfButtonSize.small,
              busy: _busy.value,
              onPressed: () => unawaited(_save()),
            ),
          ),
        ),
      ],
    ),
  );
}

const _qualities = [
  (null, 'Auto', 'Whatever this device can play'),
  (20000000, '20 Mbps', null),
  (8000000, '8 Mbps', null),
  (4000000, '4 Mbps', 'Data saver'),
];

class _ThisDevicePanel extends SignalWidget {
  const _ThisDevicePanel({required this.controller});
  final AccountController controller;

  Future<void> _chooseQuality(PlaybackService playback) async {
    // A sheet dismissed answers null, which can't also mean Auto.
    const auto = -1;
    final choice = await pickOption<int>(
      title: 'Streaming quality',
      selected: playback.maxBitrateBps.value ?? auto,
      options: [
        for (final (value, label, detail) in _qualities)
          NfOption(value ?? auto, label, detail: detail),
      ],
    );
    if (choice == null) return;
    await playback.setMaxBitrate(choice == auto ? null : choice);
  }

  @override
  Widget build(BuildContext context) {
    final playback = getIt<PlaybackService>();
    final limit = playback.maxBitrateBps.value;
    final label = _qualities
        .firstWhere((q) => q.$1 == limit, orElse: () => _qualities.first)
        .$2;
    final cap = controller.streamCapBps.value;
    final profile = getIt<DecoderProbeService>().profile.value;
    final remeasuring = controller.remeasuring.value;
    return AccountPanel(
      icon: PhosphorIconsRegular.gauge,
      title: 'On this device',
      detail: 'These stay here, and don’t follow your account',
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          InkWell(
            onTap: () => unawaited(_chooseQuality(playback)),
            borderRadius: BorderRadius.circular(NfRadii.s),
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 8),
              child: Row(
                children: [
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          'Streaming quality',
                          style: nfText(14.5, weight: 550),
                        ),
                        const SizedBox(height: 2),
                        Text(
                          cap == null
                              ? label
                              : '$label · your server caps streams at '
                                    '${(cap / 1000000).toStringAsFixed(cap % 1000000 == 0 ? 0 : 1)} Mbps',
                          style: NfType.caption,
                        ),
                      ],
                    ),
                  ),
                  const Icon(
                    PhosphorIconsRegular.caretRight,
                    size: 16,
                    color: NfColors.text3,
                  ),
                ],
              ),
            ),
          ),
          const Divider(height: 20),
          Row(
            children: [
              Expanded(
                child: Text(
                  'What this device decodes',
                  style: nfText(14.5, weight: 550),
                ),
              ),
              NfButton(
                label: 'Measure again',
                style: NfButtonStyle.ghost,
                size: NfButtonSize.small,
                busy: remeasuring,
                onPressed: profile == null
                    ? null
                    : () => unawaited(controller.remeasureDecoders()),
              ),
            ],
          ),
          const SizedBox(height: 8),
          if (profile == null)
            Text('Measuring…', style: NfType.small)
          else ...[
            for (final codec in measuredCodecs)
              _CodecRow(codec: codec, profile: profile),
            if (profile.hwdecApi != null) ...[
              const SizedBox(height: 6),
              Text(
                'Measured through ${profile.hwdecApi}',
                style: NfType.caption,
              ),
            ],
          ],
        ],
      ),
    );
  }
}

class _CodecRow extends StatelessWidget {
  const _CodecRow({required this.codec, required this.profile});
  final String codec;
  final DecoderProfile profile;

  @override
  Widget build(BuildContext context) {
    final (:hardware, :detail) = decoderSummary(codec, profile);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 3),
      child: Row(
        children: [
          Icon(
            hardware ? PhosphorIconsFill.checkCircle : PhosphorIconsRegular.cpu,
            size: 15,
            color: hardware ? NfColors.success : NfColors.text3,
          ),
          const SizedBox(width: 8),
          SizedBox(
            width: 56,
            child: Text(codecName(codec), style: nfText(13, weight: 600)),
          ),
          Expanded(child: Text(detail, style: NfType.small)),
        ],
      ),
    );
  }
}
