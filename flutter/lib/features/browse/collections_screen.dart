// SPDX-License-Identifier: AGPL-3.0-only
//
// Library collections and the viewer's playlists, as shelves opening a paged
// grid. Curating collections, and what goes in a list, stays on the web; a
// playlist is filled from a title's own page.

import 'dart:async';

import 'package:ferry/ferry.dart';
import 'package:flutter/material.dart';
import 'package:phosphor_icons/phosphor_icons.dart';
import 'package:signals_flutter/signals_flutter.dart';

import '../../di.dart';
import '../../graphql/queries/__generated__/collection_items.req.gql.dart';
import '../../graphql/queries/__generated__/collection_shelves.req.gql.dart';
import '../../graphql/queries/__generated__/create_playlist.req.gql.dart';
import '../../graphql/queries/__generated__/delete_playlist.req.gql.dart';
import '../../graphql/queries/__generated__/playlist_items.req.gql.dart';
import '../../graphql/queries/__generated__/playlist_shelves.req.gql.dart';
import '../../graphql/query.dart';
import '../../layout/window.dart';
import '../../models/work.dart';
import '../../services/graphql.dart';
import '../../services/nav_service.dart';
import '../../services/session.dart';
import '../../services/toast_service.dart';
import '../../theme.dart';
import '../../widgets/nf_button.dart';
import '../../widgets/nf_field.dart';
import '../../widgets/nf_overlays.dart';
import '../../widgets/nf_state.dart';
import '../../widgets/rail.dart';
import '../../widgets/work_card.dart';
import '../../widgets/work_grid.dart';
import '../player/playback_service.dart';
import 'work_browser.dart';

enum _Tab { collections, playlists }

/// A collection or playlist, the same from a shelf's point of view.
class _Shelf {
  const _Shelf({
    required this.id,
    required this.title,
    required this.overview,
    required this.works,
    required this.hasMore,
    required this.page,
  });

  final String id;
  final String title;
  final String? overview;
  final List<WorkSummary> works;
  final bool hasMore;
  final Future<(List<WorkSummary>, String?)> Function(String? after) page;

  String get caption {
    if (overview?.isNotEmpty ?? false) return overview!;
    final count = works.length;
    return '$count${hasMore ? '+' : ''} ${count == 1 && !hasMore ? 'title' : 'titles'}';
  }
}

Future<(List<_Shelf>, String?)> _collectionShelves(
  String library,
  String? after,
) async {
  final data = await getIt<GraphqlService>().run(
    GCollectionShelvesReq(
      (b) => b
        ..fetchPolicy = FetchPolicy.NetworkOnly
        ..vars.library = library
        ..vars.after = after,
    ),
  );
  final info = data.collections.pageInfo;
  return (
    [
      for (final node in data.collections.nodes)
        _Shelf(
          id: node.id,
          title: node.title,
          overview: node.overview,
          works: [
            for (final work in node.items.nodes) WorkSummary.fromCard(work),
          ],
          hasMore: node.items.pageInfo.hasNextPage,
          page: (after) => _collectionPage(node.id, after),
        ),
    ],
    info.hasNextPage ? info.endCursor : null,
  );
}

Future<(List<_Shelf>, String?)> _playlistShelves(String? after) async {
  final data = await getIt<GraphqlService>().run(
    GPlaylistShelvesReq(
      (b) => b
        ..fetchPolicy = FetchPolicy.NetworkOnly
        ..vars.after = after,
    ),
  );
  final info = data.playlists.pageInfo;
  return (
    [
      for (final node in data.playlists.nodes)
        _Shelf(
          id: node.id,
          title: node.title,
          overview: node.overview,
          works: [
            for (final item in node.items.nodes)
              if (item.work case final work?) WorkSummary.fromCard(work),
          ],
          hasMore: node.items.pageInfo.hasNextPage,
          page: (after) => _playlistPage(node.id, after),
        ),
    ],
    info.hasNextPage ? info.endCursor : null,
  );
}

Future<(List<WorkSummary>, String?)> _collectionPage(
  String id,
  String? after,
) async {
  final data = await getIt<GraphqlService>().run(
    GCollectionItemsReq(
      (b) => b
        ..fetchPolicy = FetchPolicy.NetworkOnly
        ..vars.id = id
        ..vars.after = after,
    ),
  );
  final items = data.collection?.items;
  if (items == null) return (const <WorkSummary>[], null);
  return (
    [for (final work in items.nodes) WorkSummary.fromCard(work)],
    items.pageInfo.hasNextPage ? items.pageInfo.endCursor : null,
  );
}

/// A page can hold nothing but whole-edition entries, which this list can't
/// show, so it carries on until there's something to return. The grid only
/// asks for more when a card nearing the end gets built.
Future<(List<WorkSummary>, String?)> _playlistPage(
  String id,
  String? after,
) async {
  var cursor = after;
  while (true) {
    final current = cursor;
    final data = await getIt<GraphqlService>().run(
      GPlaylistItemsReq(
        (b) => b
          ..fetchPolicy = FetchPolicy.NetworkOnly
          ..vars.id = id
          ..vars.after = current,
      ),
    );
    final items = data.playlist?.items;
    if (items == null) return (const <WorkSummary>[], null);
    final works = [
      for (final item in items.nodes)
        if (item.work case final work?) WorkSummary.fromCard(work),
    ];
    final next = items.pageInfo.hasNextPage ? items.pageInfo.endCursor : null;
    if (works.isNotEmpty || next == null) return (works, next);
    cursor = next;
  }
}

class CollectionsScreen extends StatefulWidget {
  const CollectionsScreen({super.key});

  @override
  State<CollectionsScreen> createState() => _CollectionsScreenState();
}

class _CollectionsScreenState extends State<CollectionsScreen> {
  final _tab = signal(_Tab.collections);
  late final _library = signal(
    getIt<SessionService>().libraries.value.firstOrNull?.id,
  );
  final _collections = signal<Pager<_Shelf>?>(null);
  final _playlists = Pager<_Shelf>(_playlistShelves);

  @override
  void initState() {
    super.initState();
    _loadCollections();
    unawaited(_playlists.refresh());
  }

  void _loadCollections() {
    final library = _library.value;
    final pager = library == null
        ? null
        : Pager<_Shelf>((after) => _collectionShelves(library, after));
    _collections.value = pager;
    unawaited(pager?.refresh());
  }

  Future<void> _chooseLibrary() async {
    final choice = await pickOption<String>(
      title: 'Library',
      selected: _library.value,
      options: [
        for (final library in getIt<SessionService>().libraries.value)
          NfOption(library.id, library.name),
      ],
    );
    if (choice == null || choice == _library.value) return;
    _library.value = choice;
    _loadCollections();
  }

  Future<void> _newPlaylist() async {
    final created = await getIt<NavService>().openSheet<bool>(
      (_) => const _NewPlaylistSheet(),
    );
    if (created == true) await _playlists.refresh();
  }

  Future<void> _deletePlaylist(_Shelf shelf) async {
    final confirmed = await confirmAction(
      title: 'Delete “${shelf.title}”?',
      message: 'The titles inside stay in your library.',
      confirmLabel: 'Delete',
      danger: true,
    );
    if (!confirmed) return;
    final data = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GDeletePlaylistReq((b) => b.vars.id = shelf.id),
      ),
      success: 'Playlist deleted',
    );
    if (data != null) await _playlists.refresh();
  }

  Future<void> _refresh() async {
    final pager = _tab.value == _Tab.playlists
        ? _playlists
        : _collections.value;
    await pager?.refresh();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: const NfPageBar(title: 'Collections'),
    body: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _Toolbar(
          tab: _tab,
          library: _library,
          onChooseLibrary: () => unawaited(_chooseLibrary()),
          onNewPlaylist: () => unawaited(_newPlaylist()),
        ),
        Expanded(
          child: RefreshIndicator(
            onRefresh: _refresh,
            child: SignalBuilder(
              builder: (context) => _tab.value == _Tab.playlists
                  ? _ShelfList(
                      key: const ValueKey(_Tab.playlists),
                      pager: _playlists,
                      onDelete: (shelf) => unawaited(_deletePlaylist(shelf)),
                      empty: NfState(
                        icon: PhosphorIconsRegular.listBullets,
                        title: 'No playlists yet',
                        message: 'Make a personal queue, then add titles from their page.',
                        actionLabel: 'New playlist',
                        onAction: () => unawaited(_newPlaylist()),
                      ),
                    )
                  : _collectionsBody(),
            ),
          ),
        ),
      ],
    ),
  );

  Widget _collectionsBody() {
    final pager = _collections.value;
    if (pager == null) {
      return const _Message(
        NfState(
          icon: PhosphorIconsRegular.cardsThree,
          title: 'No libraries yet',
          message: 'Collections belong to a library, and you don’t have access to one.',
        ),
      );
    }
    final isAdmin = getIt<SessionService>().viewer.value?.isAdmin ?? false;
    return _ShelfList(
      key: ValueKey(_library.value),
      pager: pager,
      empty: NfState(
        icon: PhosphorIconsRegular.cardsThree,
        title: 'No collections in this library',
        message: isAdmin
            ? 'Collections are curated from NoFlex on the web: a franchise, a director, a mood.'
            : 'An administrator hasn’t curated any collections here yet.',
      ),
    );
  }
}

class _Toolbar extends SignalWidget {
  const _Toolbar({
    required this.tab,
    required this.library,
    required this.onChooseLibrary,
    required this.onNewPlaylist,
  });

  final Signal<_Tab> tab;
  final Signal<String?> library;
  final VoidCallback onChooseLibrary;
  final VoidCallback onNewPlaylist;

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    final libraries = getIt<SessionService>().libraries.value;
    final chosen = libraries.where((l) => l.id == library.value).firstOrNull;
    final onCollections = tab.value == _Tab.collections;
    return SizedBox(
      height: 52,
      child: Row(
        children: [
          Expanded(
            child: ListView(
              scrollDirection: Axis.horizontal,
              padding: EdgeInsets.fromLTRB(gutter, 6, gutter, 8),
              children: [
                NfChoiceChip(
                  label: 'Library collections',
                  selected: onCollections,
                  onTap: () => tab.value = _Tab.collections,
                ),
                const SizedBox(width: 8),
                NfChoiceChip(
                  label: 'My playlists',
                  selected: !onCollections,
                  onTap: () => tab.value = _Tab.playlists,
                ),
                if (onCollections && libraries.length > 1) ...[
                  const SizedBox(width: 8),
                  NfChoiceChip(
                    label: chosen?.name ?? 'Library',
                    selected: false,
                    leading: const Icon(PhosphorIconsRegular.folderSimple),
                    onTap: onChooseLibrary,
                  ),
                ],
              ],
            ),
          ),
          if (!onCollections)
            Padding(
              padding: EdgeInsets.only(right: gutter - 4, bottom: 4),
              child: NfIconButton(
                icon: PhosphorIconsRegular.plus,
                label: 'New playlist',
                onPressed: onNewPlaylist,
              ),
            ),
        ],
      ),
    );
  }
}

/// Inside a scrollable, so pull-to-refresh still works with nothing to show.
class _Message extends StatelessWidget {
  const _Message(this.child);
  final Widget child;

  @override
  Widget build(BuildContext context) => CustomScrollView(
    physics: const AlwaysScrollableScrollPhysics(),
    slivers: [SliverFillRemaining(hasScrollBody: false, child: child)],
  );
}

class _Loading extends StatelessWidget {
  const _Loading();

  @override
  Widget build(BuildContext context) => ListView(
    physics: const NeverScrollableScrollPhysics(),
    children: const [RailSkeleton(), RailSkeleton()],
  );
}

class _ShelfList extends StatelessWidget {
  const _ShelfList({
    super.key,
    required this.pager,
    required this.empty,
    this.onDelete,
  });

  final Pager<_Shelf> pager;
  final Widget empty;
  final ValueChanged<_Shelf>? onDelete;

  @override
  Widget build(BuildContext context) => SignalBuilder(
    builder: (context) {
      final shelves = pager.items.value;
      final failure = pager.error.value;
      if (shelves.isEmpty) {
        if (failure != null) {
          return _Message(
            NfState.error(
              title: 'Couldn’t load these lists',
              message: failure.toString(),
              retry: pager.refresh,
            ),
          );
        }
        if (pager.loading.value || pager.hasMore.value) return const _Loading();
        return _Message(empty);
      }
      final hasMore = pager.hasMore.value;
      return ListView.builder(
        physics: const AlwaysScrollableScrollPhysics(),
        padding: const EdgeInsets.only(bottom: 32),
        itemCount: shelves.length + (hasMore ? 1 : 0),
        itemBuilder: (_, index) {
          if (index < shelves.length) {
            return _ShelfRow(shelf: shelves[index], onDelete: onDelete);
          }
          // Failing to load more leaves a button, not a loop of retries.
          if (failure != null) {
            return Padding(
              padding: const EdgeInsets.all(24),
              child: Center(
                child: NfButton(label: 'Load more', onPressed: pager.loadMore),
              ),
            );
          }
          WidgetsBinding.instance.addPostFrameCallback(
            (_) => unawaited(pager.loadMore()),
          );
          return const RailSkeleton();
        },
      );
    },
  );
}

class _ShelfRow extends StatelessWidget {
  const _ShelfRow({required this.shelf, this.onDelete});

  final _Shelf shelf;
  final ValueChanged<_Shelf>? onDelete;

  void _open() => getIt<NavService>().push(_ShelfGridScreen(shelf: shelf));

  @override
  Widget build(BuildContext context) {
    final gutter = context.gutter;
    final width = MediaQuery.sizeOf(context).width;
    final cardWidth = (width * 0.34).clamp(116.0, 176.0);
    final first = shelf.works.firstOrNull;
    return Padding(
      padding: const EdgeInsets.only(top: 24),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: EdgeInsets.symmetric(horizontal: gutter),
            child: NfSectionHead(
              shelf.title,
              onTap: _open,
              trailing: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  // A series has nothing of its own to play.
                  if (first != null && !first.isContainer)
                    NfButton(
                      label: 'Play',
                      size: NfButtonSize.small,
                      icon: const Icon(PhosphorIconsFill.play),
                      onPressed: () => getIt<PlaybackService>().playWork(first),
                    ),
                  if (onDelete != null)
                    NfIconButton(
                      icon: PhosphorIconsRegular.trash,
                      label: 'Delete ${shelf.title}',
                      size: 40,
                      iconSize: 17,
                      color: NfColors.text3,
                      onPressed: () => onDelete!(shelf),
                    ),
                ],
              ),
            ),
          ),
          Padding(
            padding: EdgeInsets.fromLTRB(gutter, 2, gutter, 10),
            child: Text(
              shelf.caption,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: NfType.small,
            ),
          ),
          if (shelf.works.isEmpty)
            Padding(
              padding: EdgeInsets.symmetric(horizontal: gutter),
              child: Text(
                'Empty for now. Add titles from the ••• menu on their page.',
                style: NfType.caption,
              ),
            )
          else
            SizedBox(
              height: cardWidth * 3 / 2 + 46,
              child: ListView.separated(
                scrollDirection: Axis.horizontal,
                padding: EdgeInsets.symmetric(horizontal: gutter),
                itemCount: shelf.works.length,
                separatorBuilder: (_, _) => const SizedBox(width: 12),
                itemBuilder: (_, index) => SizedBox(
                  width: cardWidth,
                  child: WorkCard(work: shelf.works[index]),
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _ShelfGridScreen extends StatefulWidget {
  const _ShelfGridScreen({required this.shelf});
  final _Shelf shelf;

  @override
  State<_ShelfGridScreen> createState() => _ShelfGridScreenState();
}

class _ShelfGridScreenState extends State<_ShelfGridScreen> {
  late final _pager = Pager<WorkSummary>(widget.shelf.page);

  @override
  void initState() {
    super.initState();
    unawaited(_pager.refresh());
  }

  @override
  Widget build(BuildContext context) {
    final overview = widget.shelf.overview;
    final gutter = context.gutter;
    return Scaffold(
      appBar: NfPageBar(title: widget.shelf.title),
      body: RefreshIndicator(
        onRefresh: _pager.refresh,
        child: CustomScrollView(
          slivers: [
            if (overview?.isNotEmpty ?? false)
              SliverPadding(
                padding: EdgeInsets.fromLTRB(gutter, 4, gutter, 8),
                sliver: SliverToBoxAdapter(
                  child: Text(overview!, style: NfType.bodyMuted),
                ),
              ),
            SliverWorkGrid(
              pager: _pager,
              emptyTitle: 'Nothing here yet',
              emptyMessage: 'Add titles from the ••• menu on their page.',
            ),
          ],
        ),
      ),
    );
  }
}

class _NewPlaylistSheet extends StatefulWidget {
  const _NewPlaylistSheet();

  @override
  State<_NewPlaylistSheet> createState() => _NewPlaylistSheetState();
}

class _NewPlaylistSheetState extends State<_NewPlaylistSheet> {
  final _title = TextEditingController();
  final _overview = TextEditingController();
  final _hasTitle = signal(false);
  final _saving = signal(false);

  @override
  void dispose() {
    _title.dispose();
    _overview.dispose();
    super.dispose();
  }

  Future<void> _create() async {
    final title = _title.text.trim();
    if (title.isEmpty || _saving.value) return;
    _saving.value = true;
    final overview = _overview.text.trim();
    final data = await getIt<ToastService>().attempt(
      () => getIt<GraphqlService>().run(
        GCreatePlaylistReq(
          (b) => b.vars
            ..title = title
            ..overview = overview.isEmpty ? null : overview,
        ),
      ),
      success: 'Playlist created',
    );
    if (!mounted) return;
    _saving.value = false;
    if (data != null) Navigator.of(context).pop(true);
  }

  @override
  Widget build(BuildContext context) => Padding(
    // A dialog already lifts itself above the keyboard; a sheet has to ask.
    padding: EdgeInsets.only(
      bottom: context.windowClass.isCompact
          ? MediaQuery.viewInsetsOf(context).bottom
          : 0,
    ),
    child: SafeArea(
      top: false,
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(22),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('New playlist', style: NfType.sectionTitle),
            const SizedBox(height: 16),
            NfField(
              controller: _title,
              label: 'Title',
              hint: 'Sunday night',
              autofocus: true,
              textInputAction: TextInputAction.next,
              onChanged: (value) => _hasTitle.value = value.trim().isNotEmpty,
            ),
            const SizedBox(height: 12),
            NfField(
              controller: _overview,
              label: 'Description',
              hint: 'Optional',
              maxLines: 3,
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
                SignalBuilder(
                  builder: (context) => NfButton(
                    label: 'Create',
                    style: NfButtonStyle.primary,
                    busy: _saving.value,
                    onPressed: _hasTitle.value
                        ? () => unawaited(_create())
                        : null,
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    ),
  );
}
