// SPDX-License-Identifier: AGPL-3.0-only
//
// An administrator's tools for one title: matching it to a provider, editing and
// locking its details, and removing it. Changes apply for everyone on the server,
// so this only opens on the home server.

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/__generated__/schema.schema.gql.dart';
import '../../graphql/queries/__generated__/delete_edition.req.gql.dart';
import '../../graphql/queries/__generated__/delete_media_file.req.gql.dart';
import '../../graphql/queries/__generated__/delete_work.req.gql.dart';
import '../../graphql/queries/__generated__/identify_work.req.gql.dart';
import '../../graphql/queries/__generated__/lock_work_field.req.gql.dart';
import '../../graphql/queries/__generated__/metadata_search.data.gql.dart';
import '../../graphql/queries/__generated__/metadata_search.req.gql.dart';
import '../../graphql/queries/fragments/__generated__/provider_attribution_fields.data.gql.dart';
import '../../graphql/queries/__generated__/refresh_metadata.req.gql.dart';
import '../../graphql/queries/__generated__/update_work_fields.req.gql.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../utils/format.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/provider_logo.dart';
import 'work_detail_controller.dart';

typedef _Match = GMetadataSearchData_searchMetadata;

Future<void> openManageSheet(WorkDetailController controller) =>
    getIt<NavService>().openSheet<void>(
      (context) => _ManageSheet(controller: controller),
      wide: true,
    );

class _ManageSheet extends StatefulWidget {
  const _ManageSheet({required this.controller});
  final WorkDetailController controller;

  @override
  State<_ManageSheet> createState() => _ManageSheetState();
}

class _ManageSheetState extends State<_ManageSheet> {
  WorkDetailController get _controller => widget.controller;
  GraphqlService get _graphql => getIt<GraphqlService>();
  ToastService get _toasts => getIt<ToastService>();

  final _query = TextEditingController();
  final _searchYear = TextEditingController();
  final _matches = signal<List<_Match>>(const []);
  final _credits = signal<Map<String, GProviderAttributionFields>>(const {});
  final _busy = signal<String?>(null);

  late final _title = TextEditingController();
  late final _sortTitle = TextEditingController();
  late final _originalTitle = TextEditingController();
  late final _year = TextEditingController();
  late final _releaseDate = TextEditingController();
  late final _rating = TextEditingController();
  late final _tagline = TextEditingController();
  late final _overview = TextEditingController();

  @override
  void initState() {
    super.initState();
    _fill();
  }

  /// What's saved now, to fill the form and to compare it against.
  void _fill() {
    final work = _controller.query.data.peek()?.work;
    if (work == null) return;
    _title.text = work.title;
    _sortTitle.text = work.sortTitle;
    _originalTitle.text = work.originalTitle ?? '';
    _year.text = work.year?.toString() ?? '';
    _releaseDate.text = work.releaseDate ?? '';
    _rating.text = work.communityRating?.toString() ?? '';
    _tagline.text = work.tagline ?? '';
    _overview.text = work.overview ?? '';
  }

  @override
  void dispose() {
    for (final field in [
      _query,
      _searchYear,
      _title,
      _sortTitle,
      _originalTitle,
      _year,
      _releaseDate,
      _rating,
      _tagline,
      _overview,
    ]) {
      field.dispose();
    }
    super.dispose();
  }

  Future<void> _search() async {
    final work = _controller.work.peek();
    if (work == null) return;
    _busy.value = 'search';
    final data = await _toasts.attempt(
      () => _graphql.run(
        GMetadataSearchReq(
          (b) => b.vars
            ..query = _query.text.trim().isEmpty
                ? work.title
                : _query.text.trim()
            ..kind = GWorkKind.valueOf(work.kind)
            ..year = int.tryParse(_searchYear.text.trim()) ?? work.year,
        ),
      ),
    );
    if (!mounted) return;
    _busy.value = null;
    if (data != null) {
      batch(() {
        _credits.value = {
          for (final credit in data.providerAttributions)
            credit.provider: credit,
        };
        _matches.value = data.searchMetadata.toList();
      });
    }
  }

  Future<void> _identify(_Match match) async {
    _busy.value = 'identify';
    final data = await _toasts.attempt(
      () => _graphql.run(
        GIdentifyWorkReq(
          (b) => b.vars
            ..id = _controller.workId
            ..provider = match.provider
            ..kind = match.externalId.kind
            ..value = match.externalId.value,
        ),
      ),
      success: 'Matched to “${match.title}”',
    );
    if (!mounted) return;
    _busy.value = null;
    if (data == null) return;
    _matches.value = const [];
    _controller.query.refetch();
  }

  Future<void> _refresh({required bool force}) async {
    _busy.value = force ? 'force' : 'refresh';
    final data = await _toasts.attempt(
      () => _graphql.run(
        GRefreshMetadataReq(
          (b) => b.vars
            ..id = _controller.workId
            ..force = force,
        ),
      ),
      success: 'Metadata refreshed',
    );
    if (!mounted) return;
    _busy.value = null;
    if (data != null) _controller.query.refetch();
  }

  Future<void> _save() async {
    final work = _controller.query.data.peek()?.work;
    if (work == null) return;
    String? changed(TextEditingController field, String? original) {
      final value = field.text.trim();
      return value.isEmpty || value == (original ?? '') ? null : value;
    }

    final title = changed(_title, work.title);
    final sortTitle = changed(_sortTitle, work.sortTitle);
    final originalTitle = changed(_originalTitle, work.originalTitle);
    final year = changed(_year, work.year?.toString());
    final releaseDate = changed(_releaseDate, work.releaseDate);
    final rating = changed(_rating, work.communityRating?.toString());
    final tagline = changed(_tagline, work.tagline);
    final overview = changed(_overview, work.overview);
    final fields = GWorkFieldsInput(
      (b) => b
        ..title = title
        ..sortTitle = sortTitle
        ..originalTitle = originalTitle
        ..year = year == null ? null : int.tryParse(year)
        ..releaseDate = releaseDate
        ..communityRating = rating == null ? null : double.tryParse(rating)
        ..tagline = tagline
        ..overview = overview,
    );
    if ([
      title,
      sortTitle,
      originalTitle,
      year,
      releaseDate,
      rating,
      tagline,
      overview,
    ].every((value) => value == null)) {
      return;
    }
    _busy.value = 'save';
    final data = await _toasts.attempt(
      () => _graphql.run(
        GUpdateWorkFieldsReq(
          (b) => b.vars
            ..id = _controller.workId
            ..fields.replace(fields),
        ),
      ),
      success: 'Saved. Edited fields are now locked.',
    );
    if (!mounted) return;
    _busy.value = null;
    if (data != null) _controller.query.refetch();
  }

  Future<void> _toggleLock(GWorkField field, bool locked) async {
    final data = await _toasts.attempt(
      () => _graphql.run(
        GLockWorkFieldReq(
          (b) => b.vars
            ..id = _controller.workId
            ..field = field
            ..locked = !locked,
        ),
      ),
    );
    if (data != null) _controller.query.refetch();
  }

  /// False to keep the files, true to delete them too, null to think again.
  Future<bool?> _askHowToDelete(String label) => pickOption<bool>(
    title: 'Delete $label?',
    options: const [
      NfOption(
        false,
        'Remove from the library',
        detail: 'It reappears on the next scan while the files stay on disk.',
      ),
      NfOption(
        true,
        'Also delete the files from disk',
        detail: 'This can’t be undone.',
      ),
    ],
  );

  Future<void> _deleteWork(String title) async {
    final files = await _askHowToDelete('“$title”');
    if (files == null) return;
    final data = await _toasts.attempt(
      () => _graphql.run(
        GDeleteWorkReq(
          (b) => b.vars
            ..id = _controller.workId
            ..files = files,
        ),
      ),
      success: 'Deleted',
    );
    if (data == null || !mounted) return;
    Navigator.of(context).pop();
    contentNavigatorKey.currentState?.pop();
  }

  Future<void> _deleteEdition(String id, String name) async {
    final files = await _askHowToDelete('the “$name” edition');
    if (files == null) return;
    final data = await _toasts.attempt(
      () => _graphql.run(
        GDeleteEditionReq(
          (b) => b.vars
            ..id = id
            ..files = files,
        ),
      ),
      success: 'Deleted',
    );
    if (data == null || !mounted) return;
    _controller.query.refetch();
    Navigator.of(context).pop();
  }

  Future<void> _deleteMedia(String id, int part) async {
    final files = await _askHowToDelete('part ${part + 1} of this edition');
    if (files == null) return;
    final data = await _toasts.attempt(
      () => _graphql.run(
        GDeleteMediaFileReq(
          (b) => b.vars
            ..id = id
            ..files = files,
        ),
      ),
      success: 'Deleted',
    );
    if (data == null || !mounted) return;
    _controller.query.refetch();
    Navigator.of(context).pop();
  }

  Widget _heading(IconData icon, String title) => Padding(
    padding: const EdgeInsets.only(top: 20, bottom: 12),
    child: Row(
      children: [
        Icon(icon, size: 16, color: NfColors.text3),
        const SizedBox(width: 9),
        Text(title, style: nfText(15, weight: 650)),
      ],
    ),
  );

  Widget _pair(Widget first, Widget second) => LayoutBuilder(
    builder: (context, constraints) => constraints.maxWidth < 440
        ? Column(children: [first, const SizedBox(height: 12), second])
        : Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(child: first),
              const SizedBox(width: 12),
              Expanded(child: second),
            ],
          ),
  );

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final work = _controller.work.value;
      final detail = _controller.query.data.value?.work;
      final edition = _controller.edition.value;
      final version = _controller.version.value;
      final locked = detail?.lockedFields.toSet() ?? const <GWorkField>{};
      final busy = _busy.value;
      return SingleChildScrollView(
        padding: const EdgeInsets.fromLTRB(22, 22, 22, 26),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              'Manage “${work?.title ?? 'this title'}”',
              style: NfType.sectionTitle,
            ),
            const SizedBox(height: 4),
            Text(
              'Metadata, matching and files. Changes apply for everyone on this server.',
              style: NfType.small,
            ),
            _heading(PhosphorIconsRegular.magnifyingGlass, 'Identify'),
            _pair(
              NfField(
                controller: _query,
                label: 'Title',
                hint: work?.title,
                textInputAction: TextInputAction.search,
                onSubmitted: (_) => unawaited(_search()),
              ),
              NfField(
                controller: _searchYear,
                label: 'Year',
                hint: work?.year?.toString() ?? 'Any',
                keyboardType: TextInputType.number,
                onSubmitted: (_) => unawaited(_search()),
              ),
            ),
            const SizedBox(height: 10),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                NfButton(
                  label: 'Search',
                  size: NfButtonSize.small,
                  busy: busy == 'search',
                  onPressed: () => unawaited(_search()),
                ),
                NfButton(
                  label: 'Refresh metadata',
                  size: NfButtonSize.small,
                  icon: const Icon(PhosphorIconsRegular.arrowsClockwise),
                  busy: busy == 'refresh',
                  onPressed: busy != null
                      ? null
                      : () => unawaited(_refresh(force: false)),
                ),
                NfButton(
                  label: 'Force full refresh',
                  style: NfButtonStyle.ghost,
                  size: NfButtonSize.small,
                  busy: busy == 'force',
                  onPressed: busy != null
                      ? null
                      : () => unawaited(_refresh(force: true)),
                ),
              ],
            ),
            for (final match in _matches.value)
              ListTile(
                contentPadding: EdgeInsets.zero,
                enabled: busy != 'identify',
                onTap: () => unawaited(_identify(match)),
                title: Text(match.title, style: nfText(14.5, weight: 600)),
                subtitle: _MatchSource(
                  match: match,
                  credit: _credits.value[match.provider],
                ),
                trailing: Text(
                  '${match.year ?? '—'} · ${(match.confidence * 100).round()}%',
                  style: NfType.small,
                ),
              ),
            _heading(PhosphorIconsRegular.pencilSimple, 'Edit details'),
            _pair(
              NfField(controller: _title, label: 'Title'),
              NfField(controller: _sortTitle, label: 'Sort title'),
            ),
            const SizedBox(height: 12),
            _pair(
              NfField(controller: _originalTitle, label: 'Original title'),
              NfField(
                controller: _year,
                label: 'Year',
                keyboardType: TextInputType.number,
              ),
            ),
            const SizedBox(height: 12),
            _pair(
              NfField(
                controller: _releaseDate,
                label: 'Release date',
                hint: 'YYYY-MM-DD',
              ),
              NfField(
                controller: _rating,
                label: 'Community rating',
                keyboardType: const TextInputType.numberWithOptions(
                  decimal: true,
                ),
              ),
            ),
            const SizedBox(height: 12),
            NfField(controller: _tagline, label: 'Tagline'),
            const SizedBox(height: 12),
            NfField(controller: _overview, label: 'Overview', maxLines: 5),
            const SizedBox(height: 12),
            Align(
              alignment: Alignment.centerRight,
              child: NfButton(
                label: 'Save changes',
                style: NfButtonStyle.primary,
                size: NfButtonSize.small,
                busy: busy == 'save',
                onPressed: () => unawaited(_save()),
              ),
            ),
            _heading(PhosphorIconsRegular.lock, 'Field locks'),
            Text(
              'Locked fields are never overwritten by provider refreshes.',
              style: NfType.small,
            ),
            const SizedBox(height: 10),
            Wrap(
              spacing: 6,
              runSpacing: 6,
              children: [
                for (final field in GWorkField.values)
                  NfChoiceChip(
                    label: titleCase(field.name),
                    selected: locked.contains(field),
                    onTap: () =>
                        unawaited(_toggleLock(field, locked.contains(field))),
                  ),
              ],
            ),
            _heading(PhosphorIconsRegular.trash, 'Remove'),
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                NfButton(
                  label: 'Delete title',
                  style: NfButtonStyle.danger,
                  size: NfButtonSize.small,
                  onPressed: () =>
                      unawaited(_deleteWork(work?.title ?? 'this title')),
                ),
                if (edition != null)
                  NfButton(
                    label: 'Delete edition',
                    style: NfButtonStyle.danger,
                    size: NfButtonSize.small,
                    onPressed: () =>
                        unawaited(_deleteEdition(edition.id, edition.name)),
                  ),
                if (version != null)
                  NfButton(
                    label: 'Delete media file',
                    style: NfButtonStyle.danger,
                    size: NfButtonSize.small,
                    onPressed: () =>
                        unawaited(_deleteMedia(version.id, version.partIndex)),
                  ),
              ],
            ),
          ],
        ),
      );
    },
  );
}

/// Whose match this is: the provider's logo when it has one, then its id there.
class _MatchSource extends StatelessWidget {
  const _MatchSource({required this.match, this.credit});

  final _Match match;
  final GProviderAttributionFields? credit;

  @override
  Widget build(BuildContext context) {
    final id = match.externalId.value;
    final credit = this.credit;
    if (credit?.logoUrl == null) {
      return Text(
        '${credit?.name ?? match.provider} · $id',
        style: NfType.caption,
      );
    }
    return Padding(
      padding: const EdgeInsets.only(top: 3),
      child: Row(
        children: [
          ProviderLogo(credit: credit!, height: 16),
          const SizedBox(width: 8),
          Flexible(child: Text(id, style: NfType.caption)),
        ],
      ),
    );
  }
}
