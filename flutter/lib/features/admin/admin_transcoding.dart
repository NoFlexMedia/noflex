// SPDX-License-Identifier: AGPL-3.0-only

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/admin_optimized_versions.data.gql.dart';
import '../../graphql/queries/__generated__/admin_optimized_versions.req.gql.dart';
import '../../graphql/queries/__generated__/admin_optimized_versions.var.gql.dart';
import '../../graphql/queries/__generated__/admin_smart_transcoding.data.gql.dart';
import '../../graphql/queries/__generated__/admin_smart_transcoding.req.gql.dart';
import '../../graphql/queries/__generated__/admin_smart_transcoding.var.gql.dart';
import '../../graphql/queries/__generated__/admin_tonemap_curve.data.gql.dart';
import '../../graphql/queries/__generated__/admin_tonemap_curve.req.gql.dart';
import '../../graphql/queries/__generated__/admin_tonemap_curve.var.gql.dart';
import '../../graphql/queries/__generated__/admin_transcoder.data.gql.dart';
import '../../graphql/queries/__generated__/admin_transcoder.req.gql.dart';
import '../../graphql/queries/__generated__/admin_transcoder.var.gql.dart';
import '../../graphql/queries/__generated__/set_tonemap_curve.req.gql.dart';
import '../../graphql/queries/__generated__/update_smart_transcoding.req.gql.dart';
import '../../graphql/query.dart';
import '../../services/graphql.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../account/account_widgets.dart';
import '../player/decoder_probe.dart';
import '../player/decoders.dart';
import 'admin_format.dart';
import 'admin_issues.dart';
import 'admin_widgets.dart';

typedef _Info = GAdminTranscoderData_transcoderInfo;
typedef _Settings = GAdminSmartTranscodingData_smartTranscoding;

final _curves = <(GTonemapCurve, String, String, bool)>[
  (GTonemapCurve.HABLE, 'Hable', 'Filmic, soft highlights', false),
  (
    GTonemapCurve.MOBIUS,
    'Möbius',
    'Keeps midtones, compresses highlights',
    false,
  ),
  (GTonemapCurve.REINHARD, 'Reinhard', 'Simple and even', false),
  (GTonemapCurve.SPLINE, 'Spline', 'Bright midtones', true),
  (GTonemapCurve.BT2390, 'BT.2390', 'The broadcast standard; brightest', true),
];

/// Whether this device could play a rung's encode, by the decoder measurement.
bool devicePlays(DecoderProfile profile, String codec, int pixels) {
  final name = switch (codec) {
    'H264' => 'h264',
    'H265' => 'h265',
    'AV1' => 'av1',
    'VP9' => 'vp9',
    _ => null,
  };
  if (name == null) return false;
  final hardware = profile.hardware[name] ?? const <DecodeCeiling>[];
  return hardware.any((ceiling) => ceiling.pixels >= pixels) ||
      profile.softwareCeiling.pixels >= pixels;
}

class TranscodingSection extends StatefulWidget {
  const TranscodingSection({super.key});

  @override
  State<TranscodingSection> createState() => _TranscodingSectionState();
}

class _TranscodingSectionState extends State<TranscodingSection> {
  late final Query<GAdminTranscoderData, GAdminTranscoderVars> _transcoder;
  late final Query<GAdminOptimizedVersionsData, GAdminOptimizedVersionsVars>
  _versions;
  late final Query<GAdminSmartTranscodingData, GAdminSmartTranscodingVars>
  _smart;
  late final Query<GAdminTonemapCurveData, GAdminTonemapCurveVars> _curve;
  late final Poller _versionsPoll;
  late final void Function() _stopSeed;
  final _directory = TextEditingController();
  final _storage = TextEditingController();
  final _version = TextEditingController();
  final _saving = signal(false);
  final _onlyAvailable = signal(false);
  final _openFlags = signal<Set<String>>(const {});
  _Settings? _seeded;

  @override
  void initState() {
    super.initState();
    final client = getIt<GraphqlService>().client;
    _transcoder = Query(client, GAdminTranscoderReq());
    _versions = Query(
      client,
      GAdminOptimizedVersionsReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    _smart = Query(
      client,
      GAdminSmartTranscodingReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    _curve = Query(
      client,
      GAdminTonemapCurveReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    _versionsPoll = Poller(_versions.refetch, const Duration(seconds: 5));
    _stopSeed = effect(() {
      final settings = _smart.data.value?.smartTranscoding;
      if (settings != null) untracked(() => _seed(settings));
    });
  }

  // Blank fields fall back to the server's configuration.
  void _seed(_Settings settings) {
    if (settings == _seeded) return;
    _seeded = settings;
    _directory.text = settings.directory == settings.defaultDirectory
        ? ''
        : settings.directory;
    _storage.text = settings.maxStorageGb == settings.defaultMaxStorageGb
        ? ''
        : '${settings.maxStorageGb}';
    final version = settings.maxVersionGb;
    _version.text = version == null || version == settings.defaultMaxVersionGb
        ? ''
        : '$version';
  }

  @override
  void dispose() {
    _versionsPoll.cancel();
    _stopSeed();
    _transcoder.dispose();
    _versions.dispose();
    _smart.dispose();
    _curve.dispose();
    _directory.dispose();
    _storage.dispose();
    _version.dispose();
    super.dispose();
  }

  Future<void> _saveSettings() async {
    _saving.value = true;
    final directory = _directory.text.trim();
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GUpdateSmartTranscodingReq(
          (b) => b.vars
            ..directory = directory.isEmpty ? null : directory
            ..storage = int.tryParse(_storage.text.trim())
            ..version = int.tryParse(_version.text.trim()),
        ),
      ),
      success: 'Smart transcoding settings saved',
    );
    if (!mounted) return;
    _saving.value = false;
    if (done != null) _smart.refetch();
  }

  Future<void> _chooseCurve(GTonemapCurve curve) async {
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GSetTonemapCurveReq((b) => b.vars.curve = curve),
      ),
      success: 'Tone-mapping curve saved',
    );
    if (done != null && mounted) _curve.refetch();
  }

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      if (_transcoder.waiting.value) {
        return const Padding(
          padding: EdgeInsets.all(40),
          child: Center(child: CircularProgressIndicator()),
        );
      }
      final info = _transcoder.data.value?.transcoderInfo;
      if (info == null) {
        return NfState.error(
          title: 'Couldn’t read the transcoder’s state',
          message: _transcoder.error.value?.toString(),
          retry: _transcoder.refetch,
        );
      }
      final profile = getIt<DecoderProbeService>().profile.value;
      return AdminStack(
        children: [
          AdminColumns(
            children: [
              _binary(
                'ffmpeg',
                version: info.ffmpeg.version,
                error: info.ffmpeg.error,
                path: info.ffmpeg.path,
                command: info.ffmpeg.command,
                overrideEnv: info.ffmpeg.overrideEnv,
                buildFlags: info.ffmpeg.buildFlags.toList(),
              ),
              _binary(
                'ffprobe',
                version: info.ffprobe.version,
                error: info.ffprobe.error,
                path: info.ffprobe.path,
                command: info.ffprobe.command,
                overrideEnv: info.ffprobe.overrideEnv,
                buildFlags: info.ffprobe.buildFlags.toList(),
              ),
            ],
          ),
          _smartPanel(),
          _accelerationPanel(info),
          _ladderPanel(info, profile),
          Row(
            children: [
              Expanded(
                child: Text(
                  'Codecs in this ffmpeg',
                  style: nfText(16, weight: 650),
                ),
              ),
              SizedBox(
                width: 170,
                child: AdminCheck(
                  label: 'Available only',
                  value: _onlyAvailable.value,
                  onChanged: (value) => _onlyAvailable.value = value,
                ),
              ),
            ],
          ),
          AdminColumns(
            children: [
              _codecPanel(
                'Encoders',
                'Everything the ladder can map to.',
                PhosphorIconsRegular.filmStrip,
                [
                  for (final item in info.encoders)
                    (item.name, item.media.name, item.backend, item.available),
                ],
              ),
              _codecPanel(
                'Decoders',
                'Common source formats, including GPU decoders.',
                PhosphorIconsRegular.filmReel,
                [
                  for (final item in info.decoders)
                    (item.name, item.media.name, item.backend, item.available),
                ],
              ),
            ],
          ),
        ],
      );
    },
  );

  Widget _binary(
    String name, {
    required String? version,
    required String? error,
    required String? path,
    required String command,
    required String overrideEnv,
    required List<String> buildFlags,
  }) {
    final open = _openFlags.value.contains(name);
    return AccountPanel(
      icon: PhosphorIconsRegular.filmStrip,
      title: name,
      detail:
          version?.replaceFirst(RegExp(r'\s+Copyright.*$'), '') ??
          'Version unknown',
      trailing: StatePill(
        error != null ? 'Unavailable' : 'Found',
        tone: error != null ? StateTone.bad : StateTone.ok,
      ),
      child: AdminStack(
        children: [
          if (error != null) Notice(error),
          Facts([
            Fact('Resolved path', path ?? '—', mono: true, full: true),
            Fact('Invoked as', command, mono: true),
            Fact('Override with', overrideEnv, mono: true),
          ]),
          if (buildFlags.isNotEmpty)
            Align(
              alignment: Alignment.centerLeft,
              child: NfButton(
                label: '${buildFlags.length} build flags',
                style: NfButtonStyle.ghost,
                size: NfButtonSize.small,
                icon: Icon(
                  open
                      ? PhosphorIconsRegular.caretUp
                      : PhosphorIconsRegular.caretDown,
                ),
                onPressed: () => _openFlags.value = open
                    ? ({..._openFlags.value}..remove(name))
                    : {..._openFlags.value, name},
              ),
            ),
          if (open)
            Wrap(
              spacing: 5,
              runSpacing: 5,
              children: [
                for (final flag in buildFlags)
                  TagChip(flag.replaceFirst('--enable-', ''), mono: true),
              ],
            ),
        ],
      ),
    );
  }

  Widget _smartPanel() {
    final settings = _smart.data.value?.smartTranscoding;
    final versions =
        _versions.data.value?.optimizedVersions.toList() ?? const [];
    int count(GOptimizedVersionStatus status) =>
        versions.where((v) => v.status == status).length;
    final preparedBytes = versions.fold<int>(
      0,
      (total, v) =>
          total +
          (v.status == GOptimizedVersionStatus.READY ? v.sizeBytes ?? 0 : 0),
    );
    final preparedGb = preparedBytes / (1024 * 1024 * 1024);
    final active = versions.where(
      (v) =>
          v.status == GOptimizedVersionStatus.RUNNING ||
          v.status == GOptimizedVersionStatus.FAILED,
    );
    return AccountPanel(
      icon: PhosphorIconsRegular.stack,
      title: 'Smart transcoding',
      detail:
          '${count(GOptimizedVersionStatus.READY)} ready · ${preparedGb.toStringAsFixed(1)}${settings == null ? '' : ' of ${settings.maxStorageGb}'} GB · ${count(GOptimizedVersionStatus.QUEUED)} queued',
      child: AdminStack(
        children: [
          if (settings != null && !settings.enabled)
            const Notice(
              'Turned off by transcoder.optimize.enabled in the server’s configuration.',
              warn: true,
            ),
          if (settings != null) ...[
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                NfField(
                  controller: _directory,
                  label: 'Folder',
                  hint: settings.defaultDirectory,
                  autocorrect: false,
                ),
                const FieldNote(
                  'Versions take real disk, so avoid cache volumes. Moving this leaves existing versions where they are, still playing.',
                ),
              ],
            ),
            AdminColumns(
              minWidth: 240,
              children: [
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    NfField(
                      controller: _storage,
                      label: 'Total storage (GB)',
                      hint: '${settings.defaultMaxStorageGb}',
                      keyboardType: TextInputType.number,
                    ),
                    const FieldNote(
                      'Past this, the least recently used are removed.',
                    ),
                  ],
                ),
                Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    NfField(
                      controller: _version,
                      label: 'Largest version (GB)',
                      hint: settings.defaultMaxVersionGb == null
                          ? 'No limit'
                          : '${settings.defaultMaxVersionGb}',
                      keyboardType: TextInputType.number,
                    ),
                    const FieldNote(
                      'Titles estimated bigger than this play with a live transcode instead.',
                    ),
                  ],
                ),
              ],
            ),
            Align(
              alignment: Alignment.centerLeft,
              child: NfButton(
                label: 'Save',
                style: NfButtonStyle.primary,
                size: NfButtonSize.small,
                busy: _saving.value,
                onPressed: () => unawaited(_saveSettings()),
              ),
            ),
          ],
          for (final version in active)
            Wrap(
              spacing: 14,
              runSpacing: 4,
              alignment: WrapAlignment.spaceBetween,
              children: [
                Text(version.variantKey, style: NfType.mono),
                if (version.status == GOptimizedVersionStatus.RUNNING)
                  Text(
                    '${(version.progress * 100).round()}%',
                    style: NfType.small,
                  )
                else
                  Text(
                    version.error ?? 'Failed',
                    style: nfText(13, color: NfColors.danger),
                  ),
              ],
            ),
        ],
      ),
    );
  }

  Widget _accelerationPanel(_Info info) {
    final curve = _curve.data.value?.tonemapCurve;
    return AccountPanel(
      icon: PhosphorIconsRegular.sparkle,
      title: 'Hardware acceleration',
      detail: 'Resolved once at startup from transcoder.hwaccel and what ffmpeg reports.',
      trailing: StatePill(
        info.hardwareAccelerated ? 'Hardware' : 'Software only',
        tone: info.hardwareAccelerated ? StateTone.accent : StateTone.none,
      ),
      child: AdminStack(
        children: [
          if (info.fellBack)
            Notice(
              'The configured ${info.configuredHwaccel} backend wasn’t available, so the ${info.fallback} fallback picked ${backendLabel(info.resolvedBackend)}.',
              warn: true,
            ),
          Facts([
            Fact('In use', backendLabel(info.resolvedBackend)),
            Fact('Configured', info.configuredHwaccel, mono: true),
            Fact('Fallback', info.fallback, mono: true),
            Fact('VAAPI device', info.vaapiDevice ?? 'none', mono: true),
            Fact('HDR tone-mapping', info.gpuTonemap ? 'GPU (Vulkan)' : 'CPU'),
            Fact(
              'ffmpeg hwaccels',
              '',
              full: true,
              child: info.hwaccels.isEmpty
                  ? Text('None', style: NfType.small)
                  : Wrap(
                      spacing: 6,
                      runSpacing: 6,
                      children: [
                        for (final accel in info.hwaccels) TagChip(accel),
                      ],
                    ),
            ),
            Fact('Transcode cache', info.transcodeDir, mono: true, full: true),
          ]),
          if (curve != null)
            AdminSelect<GTonemapCurve>(
              label: 'Tone-mapping curve',
              options: [
                for (final (value, label, hint, gpu) in _curves)
                  if (!gpu || info.gpuTonemap)
                    NfOption(value, label, detail: hint)
                  else
                    NfOption(value, label, detail: 'Needs GPU tone-mapping'),
              ],
              value: curve,
              onChanged: (value) {
                final needsGpu = _curves
                    .firstWhere((item) => item.$1 == value)
                    .$4;
                if (needsGpu && !info.gpuTonemap) return;
                unawaited(_chooseCurve(value));
              },
            ),
        ],
      ),
    );
  }

  Widget _ladderPanel(_Info info, DecoderProfile? profile) => AccountPanel(
    icon: PhosphorIconsRegular.stack,
    title: 'Transcode ladder',
    detail: 'A re-encode uses the smallest rung of its codec that holds the picture. Live playback prefers what the hardware encoder can do, then H.264.',
    child: AdminTable(
      minWidth: 700,
      columns: const [
        AdminCol('Max picture'),
        AdminCol('Video profile', flex: true),
        AdminCol('Audio'),
        AdminCol('Container'),
        AdminCol('Bitrate', numeric: true),
        AdminCol('Encoder'),
        AdminCol('This device'),
      ],
      rows: [
        for (final rung in info.ladder)
          [
            Tooltip(
              message:
                  '${(rung.maxPixels / 1000000).toStringAsFixed(rung.maxPixels < 10000000 ? 1 : 0)} MP',
              child: cellText(pixelTier(rung.maxPixels), strong: true),
            ),
            cellText(
              '${codecLabel(rung.videoCodec.name)} ${profileName(rung.videoProfile.name)}',
            ),
            cellText(rung.audioCodec.name),
            cellMono(rung.container),
            cellText('${(rung.bitrateKbps / 1000).toStringAsFixed(1)} Mb/s'),
            Wrap(
              spacing: 6,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                cellMono(rung.encoder),
                StatePill(
                  rung.encoderAvailable ? 'available' : 'missing',
                  tone: rung.encoderAvailable ? StateTone.ok : StateTone.bad,
                ),
              ],
            ),
            profile == null
                ? cellText('—')
                : devicePlays(profile, rung.videoCodec.name, rung.maxPixels)
                ? const StatePill('plays', tone: StateTone.ok)
                : cellText('can’t decode', color: NfColors.text3),
          ],
      ],
    ),
  );

  Widget _codecPanel(
    String title,
    String detail,
    IconData icon,
    List<(String, String, String, bool)> items,
  ) {
    final groups = <String, List<(String, String, String, bool)>>{};
    for (final item in items) {
      if (_onlyAvailable.value && !item.$4) continue;
      (groups['${item.$2}:${item.$3}'] ??= []).add(item);
    }
    return AccountPanel(
      icon: icon,
      title: title,
      detail: detail,
      child: groups.isEmpty
          ? Text('Nothing available.', style: NfType.caption)
          : AdminStack(
              children: [
                for (final group in groups.values)
                  Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        '${group.first.$2 == 'VIDEO' ? 'Video' : 'Audio'} · ${backendLabel(group.first.$3)}'
                            .toUpperCase(),
                        style: NfType.eyebrow,
                      ),
                      const SizedBox(height: 7),
                      Wrap(
                        spacing: 6,
                        runSpacing: 6,
                        children: [
                          for (final (name, _, _, available) in group)
                            StatePill(
                              name,
                              tone: available ? StateTone.ok : StateTone.none,
                              icon: available
                                  ? PhosphorIconsBold.check
                                  : PhosphorIconsBold.x,
                            ),
                        ],
                      ),
                    ],
                  ),
              ],
            ),
    );
  }
}
