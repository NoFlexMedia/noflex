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
import '../../graphql/queries/__generated__/optimized_version_detail.data.gql.dart';
import '../../graphql/queries/__generated__/optimized_version_detail.req.gql.dart';
import '../../graphql/queries/__generated__/remove_optimized_version.req.gql.dart';
import '../../graphql/queries/__generated__/set_viewer_device_smart_transcode.req.gql.dart';
import '../../graphql/queries/__generated__/unmark_viewer_for_smart_transcoding.req.gql.dart';
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
import '../../services/nav_service.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../utils/device_icon.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../../widgets/user_avatar.dart';
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
  static const _pageSize = 25;

  // The cursor each page walked through starts after.
  final _cursors = signal<List<String?>>(const [null]);
  final _page = signal<GAdminOptimizedVersionsData?>(null);
  final _opened = signal<String?>(null);
  final _detail = signal<GOptimizedVersionDetailData_optimizedVersion?>(null);
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
    _smart = Query(
      client,
      GAdminSmartTranscodingReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    _curve = Query(
      client,
      GAdminTonemapCurveReq((b) => b..fetchPolicy = FetchPolicy.NoCache),
    );
    unawaited(_loadVersions());
    _versionsPoll = Poller(
      () => unawaited(_loadVersions()),
      const Duration(seconds: 5),
    );
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
        return const NfSkeletonRows();
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

  Future<void> _loadVersions() async {
    final cursor = _cursors.value.last;
    try {
      final data = await getIt<GraphqlService>().run(
        GAdminOptimizedVersionsReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NetworkOnly
            ..vars.after = cursor,
        ),
      );
      if (!mounted || cursor != _cursors.value.last) return;
      // Removing the last row of the last page leaves nothing to stand on.
      if (data.optimizedVersions.nodes.isEmpty && _cursors.value.length > 1) {
        _goTo(_cursors.value.sublist(0, _cursors.value.length - 1));
        return;
      }
      _page.value = data;
    } on Exception {
      // The next tick asks again.
    }
  }

  void _goTo(List<String?> cursors) {
    _cursors.value = cursors;
    _opened.value = null;
    unawaited(_loadVersions());
  }

  Future<void> _loadDetail(String id) async {
    try {
      final data = await getIt<GraphqlService>().run(
        GOptimizedVersionDetailReq(
          (b) => b
            ..fetchPolicy = FetchPolicy.NetworkOnly
            ..vars.id = id,
        ),
      );
      if (mounted) _detail.value = data.optimizedVersion;
    } on Exception catch (error) {
      getIt<ToastService>().show(error.toString(), kind: ToastKind.error);
    }
  }

  void _toggle(String id) {
    if (_opened.value == id) {
      _opened.value = null;
      return;
    }
    _opened.value = id;
    _detail.value = null;
    unawaited(_loadDetail(id));
  }

  Future<void> _unmark(
    String version,
    GOptimizedVersionDetailData_optimizedVersion_marks mark,
  ) async {
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GUnmarkViewerForSmartTranscodingReq(
          (b) => b.vars
            ..user = mark.user.id
            ..work = mark.work.id
            ..device = mark.device?.id,
        ),
      ),
      success: 'Unmarked for ${mark.user.name}',
    );
    if (done != null) await _loadDetail(version);
  }

  Future<void> _remove(
    GAdminOptimizedVersionsData_optimizedVersions_nodes version,
  ) async {
    _opened.value = null;
    _detail.value = null;
    unawaited(_loadDetail(version.id));
    final title = version.work?.title ?? 'this title';
    final remove = await getIt<NavService>().openSheet<bool>(
      (context) => Padding(
        padding: const EdgeInsets.all(22),
        child: SingleChildScrollView(
          child: SignalBuilder(
            builder: (context) {
              final detail = _detail.value;
              return Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'Remove this copy of $title?',
                    style: NfType.sectionTitle,
                  ),
                  const SizedBox(height: 8),
                  if (version.sizeBytes != null)
                    Text(
                      'Frees ${bytes(version.sizeBytes)}.',
                      style: NfType.bodyMuted,
                    )
                  else
                    Text('Stops it being prepared.', style: NfType.bodyMuted),
                  const SizedBox(height: 12),
                  if (detail == null)
                    const SizedBox.square(
                      dimension: 20,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  else if (detail.marks.isNotEmpty ||
                      detail.targets.any(
                        (target) => target.reason == GOptimizedReason.PREDICTED,
                      )) ...[
                    Text(
                      'It’ll be prepared again while these still want it. Unmark anyone you don’t want it prepared for. Predicted devices have no mark to take back, so it may return for them.',
                      style: NfType.small,
                    ),
                    const SizedBox(height: 10),
                    _audience(version.id, detail),
                  ] else
                    Text(
                      'Nothing wants it now, so it won’t come back unless someone asks.',
                      style: NfType.small,
                    ),
                  const SizedBox(height: 20),
                  Row(
                    mainAxisAlignment: MainAxisAlignment.end,
                    children: [
                      NfButton(
                        label: 'Cancel',
                        style: NfButtonStyle.ghost,
                        onPressed: () => Navigator.of(context).pop(false),
                      ),
                      const SizedBox(width: 10),
                      NfButton(
                        label: 'Remove',
                        style: NfButtonStyle.danger,
                        onPressed: () => Navigator.of(context).pop(true),
                      ),
                    ],
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
    if (remove != true) return;
    await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GRemoveOptimizedVersionReq((b) => b.vars.id = version.id),
      ),
      success: 'Removed',
    );
    await _loadVersions();
  }

  Future<void> _turnOff(String version, String device, String name) async {
    final done = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GSetViewerDeviceSmartTranscodeReq(
          (b) => b.vars
            ..id = device
            ..enabled = false,
        ),
      ),
      success: 'Smart transcoding off on $name',
    );
    if (done != null) await _loadDetail(version);
  }

  /// Each viewer once, however many of their devices and marks it's for.
  Widget _audience(
    String version,
    GOptimizedVersionDetailData_optimizedVersion detail,
  ) {
    final viewers = <String, _Viewer>{};
    for (final target in detail.targets) {
      final user = target.user;
      (viewers[user.id] ??= _Viewer(
        user.name,
        user.username,
        user.avatarUrl,
      )).targets.add(target);
    }
    for (final mark in detail.marks) {
      final user = mark.user;
      (viewers[user.id] ??= _Viewer(
        user.name,
        user.username,
        user.avatarUrl,
      )).marks.add(mark);
    }
    String reason(GOptimizedReason value) => switch (value) {
      GOptimizedReason.MARKED => 'Marked to watch next',
      GOptimizedReason.PREDICTED => 'Predicted, with smart transcoding on',
      _ => 'Asked for earlier, not any more',
    };
    final since = detail.unwantedSince;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      spacing: 10,
      children: [
        if (since != null)
          Text(
            'Nothing’s wanted this since ${ago(since)}. It’s swept up '
            'a day after that, unless someone plays it.',
            style: NfType.small,
          )
        else if (viewers.isEmpty)
          Text('Nothing wants this any more.', style: NfType.small),
        for (final viewer in viewers.values)
          Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            spacing: 4,
            children: [
              Row(
                spacing: 10,
                children: [
                  UserAvatar(name: viewer.name, avatarUrl: viewer.avatarUrl),
                  Flexible(
                    child: Text(
                      viewer.name,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: NfType.label,
                    ),
                  ),
                  if (viewer.username != viewer.name)
                    Flexible(
                      child: Text(
                        viewer.username,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: NfType.caption,
                      ),
                    ),
                ],
              ),
              for (final row in viewer.targets)
                _audienceRow(
                  icon: row.device == null
                      ? PhosphorIconsRegular.devices
                      : deviceIconFor(row.device!.name),
                  title: row.device?.name ?? 'No device to reach',
                  detail: reason(row.reason),
                  action:
                      row.reason == GOptimizedReason.PREDICTED &&
                          row.device != null
                      ? NfButton(
                          label: 'Turn off',
                          style: NfButtonStyle.ghost,
                          size: NfButtonSize.small,
                          onPressed: () => unawaited(
                            _turnOff(version, row.device!.id, row.device!.name),
                          ),
                        )
                      : null,
                ),
              for (final mark in viewer.marks)
                _audienceRow(
                  icon: PhosphorIconsRegular.bookmarkSimple,
                  title: 'Marked ${mark.work.title}',
                  detail: mark.device == null
                      ? 'For all their devices'
                      : 'For ${mark.device!.name}',
                  action: NfButton(
                    label: 'Unmark',
                    style: NfButtonStyle.ghost,
                    size: NfButtonSize.small,
                    onPressed: () => unawaited(_unmark(version, mark)),
                  ),
                ),
            ],
          ),
      ],
    );
  }

  Widget _audienceRow({
    required IconData icon,
    required String title,
    required String detail,
    Widget? action,
  }) => Row(
    spacing: 10,
    children: [
      // Under the avatar's centre, so a viewer's rows read as theirs.
      SizedBox(width: 32, child: Icon(icon, size: 17, color: NfColors.text2)),
      Expanded(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(title, style: nfText(13.5, weight: 500)),
            Text(detail, style: NfType.caption),
          ],
        ),
      ),
      ?action,
    ],
  );

  Widget _versionRow(
    GAdminOptimizedVersionsData_optimizedVersions_nodes version,
  ) {
    final detail = _detail.value;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Wrap(
          spacing: 14,
          runSpacing: 4,
          alignment: WrapAlignment.spaceBetween,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  version.work?.title ?? 'A removed title',
                  style: NfType.label,
                ),
                Text(version.description, style: NfType.small),
              ],
            ),
            switch (version.status) {
              GOptimizedVersionStatus.RUNNING => Text(
                'Preparing ${(version.progress * 100).round()}%',
                style: NfType.small,
              ),
              GOptimizedVersionStatus.FAILED => Text(
                version.error ?? 'Failed',
                style: nfText(13, color: NfColors.danger),
              ),
              GOptimizedVersionStatus.QUEUED => Text(
                'In the queue',
                style: NfType.small,
              ),
              _ => Text(
                [
                  'Ready',
                  bytes(version.sizeBytes),
                  if (version.lastUsedAt case final played?)
                    'last played ${ago(played)}'
                  else
                    'not played yet',
                ].join(' · '),
                style: NfType.small,
              ),
            },
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                NfButton(
                  label: 'Who it’s for',
                  style: NfButtonStyle.ghost,
                  size: NfButtonSize.small,
                  onPressed: () => _toggle(version.id),
                ),
                NfButton(
                  label: 'Remove',
                  style: NfButtonStyle.ghost,
                  size: NfButtonSize.small,
                  onPressed: () => unawaited(_remove(version)),
                ),
              ],
            ),
          ],
        ),
        if (_opened.value == version.id)
          Padding(
            padding: const EdgeInsets.only(top: 8),
            child: detail == null
                ? const Align(
                    alignment: Alignment.centerLeft,
                    child: SizedBox.square(
                      dimension: 20,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                  )
                : _audience(version.id, detail),
          ),
      ],
    );
  }

  Widget _smartPanel() {
    final settings = _smart.data.value?.smartTranscoding;
    final page = _page.value;
    final stats = page?.optimizedVersionStats;
    final versions = page?.optimizedVersions.nodes.toList() ?? const [];
    final preparedGb = (stats?.readyBytes ?? 0) / (1024 * 1024 * 1024);
    final total = stats == null
        ? 0
        : stats.ready + stats.queued + stats.running + stats.failed;
    final cursors = _cursors.value;
    final pageStart = (cursors.length - 1) * _pageSize;
    final hasNext = page?.optimizedVersions.pageInfo.hasNextPage ?? false;
    return AccountPanel(
      icon: PhosphorIconsRegular.stack,
      title: 'Smart transcoding',
      detail:
          '${stats?.ready ?? 0} ready · ${preparedGb.toStringAsFixed(1)}${settings == null ? '' : ' of ${settings.maxStorageGb}'} GB · ${stats?.queued ?? 0} queued',
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
          if (page != null && versions.isEmpty)
            Text(
              'Nothing prepared yet. Versions appear here once viewers mark titles or turn smart transcoding on for a device.',
              style: NfType.small,
            ),
          for (final version in versions) _versionRow(version),
          if (total > _pageSize || cursors.length > 1)
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                Text(
                  '${pageStart + 1}–${pageStart + versions.length} of $total',
                  style: NfType.small,
                ),
                const SizedBox(width: 12),
                NfButton(
                  label: 'Previous',
                  style: NfButtonStyle.ghost,
                  size: NfButtonSize.small,
                  onPressed: cursors.length == 1
                      ? null
                      : () => _goTo(cursors.sublist(0, cursors.length - 1)),
                ),
                NfButton(
                  label: 'Next',
                  style: NfButtonStyle.ghost,
                  size: NfButtonSize.small,
                  onPressed: hasNext
                      ? () => _goTo([
                          ...cursors,
                          page!.optimizedVersions.pageInfo.endCursor,
                        ])
                      : null,
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

/// One viewer's share of what wants a version.
class _Viewer {
  _Viewer(this.name, this.username, this.avatarUrl);
  final String name;
  final String username;
  final String? avatarUrl;
  final targets = <GOptimizedVersionDetailData_optimizedVersion_targets>[];
  final marks = <GOptimizedVersionDetailData_optimizedVersion_marks>[];
}
