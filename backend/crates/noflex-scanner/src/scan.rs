// SPDX-License-Identifier: AGPL-3.0-only
//
// The one-shot scan pipeline.
//
// Each candidate gets a scan key. Movies, music and photos are keyed by
// their path under the library root, so two paths that point at the same
// canonical work stay separate candidates and merging is the metadata
// provider's job. Seasons and episodes are keyed by show name and number
// instead: quality-split season folders (`Season 01 (1080p)`,
// `Season 01 (4K)`) hold versions of the same episodes, not new ones.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use noflex_core::{
    DomainEvent, DomainEventBus, EditionId, ExternalId, ExternalIdKind, HdrTransfer, ImageKind,
    LibraryId, MetadataStatus, SubtitleCodec, WorkId, WorkKind,
};
use noflex_storage::{
    Storage,
    domain::{
        AudioTrack, Edition, Image, ImageSource, ImageTarget, Library, LibraryKind, MediaFile,
        ScanEvent, SubtitleTrack, Work,
    },
    query::{ListWorksQuery, SortOrder},
    store::atomic,
};
use noflex_transcoder::probe::ProbeReport;
use time::OffsetDateTime;
use tokio::sync::Mutex;

use crate::{
    ScanSummary,
    artwork::{ArtClaims, ArtRole},
    parser::{
        EpisodeClassification, MovieClassification, MovieIdentity, MusicClassification,
        PhotoClassification, classify_episode, classify_movie, classify_music, classify_photo,
        parse_season_folder, slugify,
    },
    walker::{MediaFileCandidate, normalize_excluded_folder, walk_media},
};

/// Version of the path-to-scan-key derivation. A library recorded under an
/// older version has every file re-planned on its next scan, so files that
/// are skipped as unchanged still move onto the works the current keys
/// name. Bump it whenever a key format, or the classification feeding one,
/// changes.
///
/// 1: season and episode keys drop the season-folder path; scene-named
///    episode files classify.
const SCAN_VERSION: u32 = 1;

static SCAN_LOCKS: LazyLock<Mutex<HashMap<LibraryId, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) struct ScanRun<'a> {
    storage: &'a dyn Storage,
    library: Library,
    library_roots: Vec<PathBuf>,
    excluded_folders: Vec<PathBuf>,
    started_at: OffsetDateTime,
    existing: HashMap<String, MediaFile>,
    edition_cache: HashMap<(WorkId, String), EditionId>,
    summary: ScanSummary,
    events: Option<DomainEventBus>,
    /// Works created / touched / pruned this run, for the `ScanCompleted`
    /// event.
    added_works: HashSet<WorkId>,
    updated_works: HashSet<WorkId>,
    removed_works: HashSet<WorkId>,
    /// The library's works predate [`SCAN_VERSION`]: re-plan unchanged
    /// files too.
    replan: bool,
    /// An unchanged file's re-plan failed, so the library must not be
    /// recorded at [`SCAN_VERSION`] yet.
    replan_failed: bool,
    /// Local-artwork claims from every walked file, changed or not.
    art: ArtClaims,
}

impl<'a> ScanRun<'a> {
    pub(crate) async fn new(
        storage: &'a dyn Storage,
        library_id: LibraryId,
        events: Option<&DomainEventBus>,
    ) -> anyhow::Result<Self> {
        let libs = storage.list_libraries().await?;
        let library = libs
            .into_iter()
            .find(|l| l.id == library_id)
            .ok_or_else(|| anyhow::anyhow!("library {library_id} not found"))?;

        let library_roots: Vec<PathBuf> = library.paths.iter().map(PathBuf::from).collect();
        let excluded_folders = library
            .config
            .excluded_folders
            .iter()
            .map(|path| normalize_excluded_folder(path).map_err(anyhow::Error::msg))
            .collect::<anyhow::Result<Vec<_>>>()?;

        let existing = storage
            .list_media_files_for_library(library.id)
            .await?
            .into_iter()
            .map(|mf| (mf.path.clone(), mf))
            .collect();
        let replan = storage.library_scan_version(library.id).await? < SCAN_VERSION;

        Ok(Self {
            storage,
            library,
            library_roots,
            excluded_folders,
            started_at: OffsetDateTime::now_utc(),
            existing,
            edition_cache: HashMap::new(),
            summary: ScanSummary::default(),
            events: events.cloned(),
            added_works: HashSet::new(),
            updated_works: HashSet::new(),
            removed_works: HashSet::new(),
            replan,
            replan_failed: false,
            art: ArtClaims::default(),
        })
    }

    pub(crate) async fn run(mut self) -> anyhow::Result<ScanSummary> {
        if let Some(bus) = &self.events {
            bus.emit(DomainEvent::ScanStarted {
                library: self.library.id,
                library_name: self.library.name.clone(),
            });
        }
        for root in self.library_roots.clone() {
            let kind = self.library.kind;
            let excluded_folders = self.excluded_folders.clone();
            let files =
                tokio::task::spawn_blocking(move || walk_media(&root, kind, &excluded_folders))
                    .await?;
            for vf in files {
                let classified = self.classify(&vf.path);
                if let Ok(c) = &classified {
                    self.claim_artwork(&vf.path, c);
                }
                if let Err(err) = self.handle_file(&vf, classified).await {
                    tracing::warn!(path = ?vf.path, %err, "scan: file errored");
                    self.summary.errored += 1;
                }
            }
        }

        // Anything still in `existing` was not seen on disk.
        let removed: Vec<MediaFile> = self.existing.values().cloned().collect();
        for mf in removed {
            match self.storage.remove_media_file_and_prune(mf.id).await {
                Ok(pruned) => {
                    self.summary.removed += 1;
                    self.removed_works.extend(pruned.removed_works);
                }
                Err(err) => {
                    tracing::warn!(path = %mf.path, %err, "scan: failed to delete missing file");
                    self.summary.errored += 1;
                }
            }
        }

        if self.replan && !self.replan_failed {
            match self
                .storage
                .set_library_scan_version(self.library.id, SCAN_VERSION)
                .await
            {
                Ok(()) => tracing::debug!(version = SCAN_VERSION, "scan: scan version recorded"),
                Err(err) => tracing::warn!(%err, "scan: re-plan not recorded; repeats next scan"),
            }
        }

        // Artwork is cosmetic: never let it fail or count against the scan.
        if let Err(err) = self.reconcile_artwork().await {
            tracing::warn!(%err, "scan: artwork discovery failed");
        }

        let event = ScanEvent {
            library_id: self.library.id,
            started_at: self.started_at,
            finished_at: Some(OffsetDateTime::now_utc()),
            added: self.summary.added,
            updated: self.summary.updated,
            removed: self.summary.removed,
            errored: self.summary.errored,
        };
        if let Err(err) = self.storage.record_scan_event(&event).await {
            tracing::warn!(%err, "scan: failed to record scan event");
        }

        if let Some(bus) = &self.events {
            bus.emit(DomainEvent::ScanCompleted {
                library: self.library.id,
                library_name: self.library.name.clone(),
                added: u64::from(self.summary.added),
                updated: u64::from(self.summary.updated),
                removed: u64::from(self.summary.removed),
                added_works: sorted_ids(&self.added_works),
                updated_works: sorted_ids(&self.updated_works),
                removed_works: sorted_ids(&self.removed_works),
            });
        }

        Ok(self.summary)
    }

    async fn handle_file(
        &mut self,
        vf: &MediaFileCandidate,
        classified: anyhow::Result<Classified>,
    ) -> anyhow::Result<()> {
        let path_str = vf
            .path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("path is not valid UTF-8: {:?}", vf.path))?
            .to_string();

        let mtime: OffsetDateTime = vf.meta.modified()?.into();
        let size_bytes = vf.meta.len();

        let existing_mf = self.existing.remove(&path_str);
        let unchanged = existing_mf
            .as_ref()
            .is_some_and(|prev| prev.mtime == mtime && prev.size_bytes == size_bytes);
        // A stored file that no longer classifies keeps its current work.
        if unchanged && (!self.replan || classified.is_err()) {
            tracing::trace!(path = %path_str, "scan: unchanged");
            return Ok(());
        }
        let result = self
            .write_file(vf, classified, path_str, mtime, size_bytes, existing_mf)
            .await;
        if unchanged && result.is_err() {
            self.replan_failed = true;
        }
        result
    }

    /// Plan and apply the rows for a new, changed, or re-planned file.
    async fn write_file(
        &mut self,
        vf: &MediaFileCandidate,
        classified: anyhow::Result<Classified>,
        path_str: String,
        mtime: OffsetDateTime,
        size_bytes: u64,
        existing_mf: Option<MediaFile>,
    ) -> anyhow::Result<()> {
        // Plan the writes (reads on the main connection), then apply
        // them in a single `atomic` block so a media-file failure
        // rolls back the work / edition we just created.
        let Some(plan) = self
            .plan_file_target(
                vf,
                classified?,
                &path_str,
                mtime,
                size_bytes,
                existing_mf.as_ref(),
            )
            .await?
        else {
            tracing::trace!(path = %path_str, "scan: re-planned in place");
            return Ok(());
        };
        // A file whose target moved leaves its old edition behind; it and
        // any ancestry it leaves empty are pruned in the same transaction.
        let prior_edition = existing_mf.as_ref().map(|m| m.edition_id);
        // New work rows in the plan become `WorkAdded` events — but
        // only after the atomic write commits, so a rolled-back plan
        // never announces a phantom work.
        let new_works: Vec<(WorkId, String)> = if self.events.is_some() {
            plan.writes
                .iter()
                .filter_map(|op| match op {
                    WriteOp::Work(w) => Some((w.id, w.title.clone())),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        let target_work = plan.work_id;
        let planned_editions: Vec<EditionId> = plan
            .writes
            .iter()
            .filter_map(|op| match op {
                WriteOp::Edition(e) => Some(e.id),
                _ => None,
            })
            .collect();
        let committed = atomic(self.storage, |tx| {
            let writes = plan.writes;
            Box::pin(async move {
                let mut edition_aliases = HashMap::new();
                let mut pruned = Vec::new();
                for op in writes {
                    match op {
                        WriteOp::Work(w) => tx.upsert_work(&w).await?,
                        WriteOp::Edition(e) => {
                            let planned = e.id;
                            let actual = tx.upsert_edition(&e).await?;
                            edition_aliases.insert(planned, actual);
                        }
                        WriteOp::MediaFile(mut m) => {
                            if let Some(actual) = edition_aliases.get(&m.edition_id) {
                                m.edition_id = *actual;
                            }
                            tx.upsert_media_file(&m).await?;
                            if let Some(prior) = prior_edition
                                && prior != m.edition_id
                            {
                                pruned.extend(tx.prune_edition(prior).await?.removed_works);
                            }
                        }
                    }
                }
                Ok((edition_aliases, pruned))
            })
        })
        .await;
        reconcile_edition_cache(
            &mut self.edition_cache,
            &planned_editions,
            committed.as_ref().ok().map(|(aliases, _)| aliases),
        );
        let (_, pruned) = committed?;
        if !pruned.is_empty() {
            self.edition_cache
                .retain(|(work, _), _| !pruned.contains(work));
            self.removed_works.extend(pruned);
        }

        if existing_mf.is_some() {
            self.summary.updated += 1;
            tracing::debug!(path = %path_str, "scan: updated");
        } else {
            self.summary.added += 1;
            tracing::debug!(path = %path_str, "scan: added");
        }
        if let Some(bus) = &self.events {
            for (work, title) in new_works {
                // A season+episode batch repeats works across files; the
                // set dedupes for the ScanCompleted id list.
                self.added_works.insert(work);
                bus.emit(DomainEvent::WorkAdded {
                    work,
                    library: self.library.id,
                    title,
                });
            }
            if existing_mf.is_some() {
                self.updated_works.insert(target_work);
            }
        }
        Ok(())
    }

    async fn plan_file_target(
        &mut self,
        vf: &MediaFileCandidate,
        classified: Classified,
        path_str: &str,
        mtime: OffsetDateTime,
        size_bytes: u64,
        existing: Option<&MediaFile>,
    ) -> anyhow::Result<Option<TargetPlan>> {
        let target = match classified {
            Classified::Movie(c) => self.plan_movie_from(c).await?,
            Classified::Episode(c) => self.plan_episode_from(c).await?,
            Classified::Music(c) => self.plan_music_from(c).await?,
            Classified::Photo(c) => self.plan_photo_from(c).await?,
        };
        let work_id = target.work_id;
        let edition_id = target.edition_id;
        let part_index = target.part_index;

        // An unchanged file is only here to be re-planned: keep its probe,
        // and skip it entirely when its target hasn't moved.
        let unchanged =
            existing.filter(|prev| prev.mtime == mtime && prev.size_bytes == size_bytes);
        let probe = match unchanged {
            Some(prev) if prev.edition_id == edition_id && prev.part_index == part_index => {
                return Ok(None);
            }
            Some(prev) => serde_json::from_value::<ProbeReport>(prev.probe.0.clone()).ok(),
            // Best-effort post-scan probe. Failure to run ffprobe
            // (binary missing, timeout, parse error) is logged and
            // the scan continues without the codec/HDR fields.
            None => match crate::ffprobe::probe_media_file(&vf.path, None).await {
                Ok(p) => Some(p),
                Err(err) => {
                    tracing::warn!(path = %path_str, %err, "scan: ffprobe failed");
                    None
                }
            },
        };

        let id = existing.map(|m| m.id).unwrap_or_default();
        // Keep the probe on the media row too (not just overlaid
        // onto the edition) so the stream session path can decide
        // direct-play vs. transcode without re-running ffprobe.
        let probe_json = probe
            .as_ref()
            .and_then(|p| serde_json::to_value(p).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let media = MediaFile {
            id,
            edition_id,
            path: path_str.to_string(),
            part_index,
            size_bytes,
            mtime,
            container: container_from_ext(&vf.path),
            probe: probe_json.into(),
            subtitle_tracks: if matches!(
                self.library.kind,
                LibraryKind::Movies | LibraryKind::Shows | LibraryKind::Mixed
            ) {
                let mut tracks = probe
                    .as_ref()
                    .map(crate::subtitles::embedded_tracks)
                    .unwrap_or_default();
                tracks.extend(crate::subtitles::discover_sidecars(&vf.path));
                tracks
            } else {
                Vec::new()
            },
        };
        // The work + edition rows are already in `plan_target.writes`; the
        // media file row is the only thing we always write per file.
        let mut writes = target.writes;
        if let Some(p) = probe {
            // Only populate the newly created edition's typed fields;
            // existing editions are re-probed on a future full rescan.
            for op in &mut writes {
                if let WriteOp::Edition(e) = op
                    && e.id == edition_id
                {
                    *e = populate_edition_from_probe(e.clone(), &p);
                }
            }
        }
        writes.push(WriteOp::MediaFile(media));
        Ok(Some(TargetPlan { work_id, writes }))
    }

    /// Decide what kind of item `path` is. Pure: no IO.
    fn classify(&self, path: &Path) -> anyhow::Result<Classified> {
        let roots = &self.library_roots;
        let episode = |path: &Path| {
            classify_episode(path, roots)
                .map(Classified::Episode)
                .ok_or_else(|| anyhow::anyhow!("can't classify episode: {path:?}"))
        };
        match self.library.kind {
            LibraryKind::Movies => classify_movie(path, roots)
                .map(Classified::Movie)
                .ok_or_else(|| anyhow::anyhow!("can't classify movie: {path:?}")),
            LibraryKind::Shows => episode(path),
            LibraryKind::Music => classify_music(path, roots)
                .map(Classified::Music)
                .ok_or_else(|| anyhow::anyhow!("can't classify music track: {path:?}")),
            LibraryKind::Photos => classify_photo(path, roots)
                .map(Classified::Photo)
                .ok_or_else(|| anyhow::anyhow!("can't classify photo: {path:?}")),
            LibraryKind::Mixed => {
                if let Some(c) = classify_movie(path, roots)
                    && (c.parsed.year.is_some() || !c.parsed.title.is_empty())
                    && classify_episode(path, roots).is_none()
                {
                    return Ok(Classified::Movie(c));
                }
                episode(path)
            }
            other => Err(anyhow::anyhow!("unsupported library kind: {other:?}")),
        }
    }

    /// Record which directories may hold local artwork for `path`'s works.
    fn claim_artwork(&mut self, path: &Path, classified: &Classified) {
        let Some(parent) = path.parent() else {
            return;
        };
        let roots = &self.library_roots;
        let is_root = |dir: &Path| roots.iter().any(|r| r == dir);
        match classified {
            Classified::Movie(c) => {
                let key = movie_scan_key(self.library.id, &c.identity, roots);
                let in_folder = matches!(c.identity, MovieIdentity::InFolder(_));
                if let Some(role) = ArtRole::movie(path, in_folder) {
                    self.art.add(parent, role, &key);
                }
            }
            Classified::Episode(c) => {
                let keys = EpisodeKeys::new(self.library.id, c);
                if let Some(role) = ArtRole::episode(path) {
                    self.art.add(parent, role, &keys.episode);
                }
                // Without a real season folder the classifier synthesises
                // one from the file's parent, which is then the show folder.
                let real_season = c
                    .season_folder
                    .file_name()
                    .and_then(|n| n.to_str())
                    .and_then(parse_season_folder)
                    .is_some();
                let show_dir = if real_season {
                    self.art
                        .add(&c.season_folder, ArtRole::SeasonFolder, &keys.season);
                    c.season_folder.parent()
                } else {
                    Some(c.season_folder.as_path())
                };
                if let Some(show_dir) = show_dir.filter(|d| !is_root(d)) {
                    self.art.add(show_dir, ArtRole::ShowFolder, &keys.series);
                    self.art.add(
                        show_dir,
                        ArtRole::ShowFolderSeason {
                            number: c.parsed.season,
                        },
                        &keys.season,
                    );
                }
            }
            Classified::Music(c) => {
                let keys = MusicKeys::new(self.library.id, c, roots);
                // `Artist/NN - Track.ext` has no album folder of its own.
                if c.album_path != c.artist_path {
                    self.art
                        .add(&c.album_path, ArtRole::AlbumFolder, &keys.album);
                }
                self.art
                    .add(&c.artist_path, ArtRole::ArtistFolder, &keys.artist);
            }
            Classified::Photo(_) => {}
        }
    }

    /// Bring the `Local`-source `images` rows of every work seen this
    /// scan in line with the artwork on disk. A local row is only deleted
    /// when it points under a library root and no local candidate
    /// remains. Provider and generated rows are never touched; which
    /// source readers see is the library's artwork preference.
    async fn reconcile_artwork(&self) -> anyhow::Result<()> {
        if self.art.is_empty() {
            return Ok(());
        }
        let dirs = self.art.dirs();
        let listings =
            tokio::task::spawn_blocking(move || crate::artwork::list_art_dirs(dirs)).await?;
        let chosen = self.art.resolve(&listings);
        let ids = self.work_ids_by_key(&self.art.work_keys()).await?;
        let seen: HashSet<WorkId> = ids.values().copied().collect();

        let mut wanted: HashMap<(WorkId, ImageKind), String> = HashMap::new();
        for ((key, kind), path) in chosen {
            if let (Some(work), Some(path)) = (ids.get(&key), path.to_str()) {
                wanted.insert((*work, kind), path.to_string());
            }
        }

        let (mut written, mut removed) = (0u32, 0u32);
        let existing = self
            .storage
            .list_work_images_for_library(self.library.id)
            .await?;
        for image in existing {
            let ImageTarget::Work(work) = image.target else {
                continue;
            };
            if image.source != ImageSource::Local || !seen.contains(&work) {
                continue;
            }
            match wanted.remove(&(work, image.kind)) {
                Some(path) if path == image.path => {}
                Some(path) => written += u32::from(self.put_image(work, image.kind, path).await),
                None if self.under_library_root(&image.path) => {
                    match self
                        .storage
                        .delete_image(image.target, image.kind, ImageSource::Local)
                        .await
                    {
                        Ok(_) => removed += 1,
                        Err(err) => {
                            tracing::warn!(path = %image.path, %err, "scan: stale artwork not removed");
                        }
                    }
                }
                None => {}
            }
        }
        for ((work, kind), path) in wanted {
            written += u32::from(self.put_image(work, kind, path).await);
        }
        if written > 0 || removed > 0 {
            tracing::debug!(written, removed, "scan: local artwork reconciled");
        }
        Ok(())
    }

    /// Returns whether the row was written; failures are logged.
    async fn put_image(&self, work: WorkId, kind: ImageKind, path: String) -> bool {
        let image = Image {
            target: ImageTarget::Work(work),
            kind,
            source: ImageSource::Local,
            path,
            width: None,
            height: None,
            language: None,
        };
        match self.storage.upsert_image(&image).await {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!(path = %image.path, %err, "scan: artwork not recorded");
                false
            }
        }
    }

    fn under_library_root(&self, path: &str) -> bool {
        let path = Path::new(path);
        self.library_roots.iter().any(|r| path.starts_with(r))
    }

    /// Ids of this library's works carrying one of `keys` as their scan
    /// key, read in pages rather than one lookup per key.
    async fn work_ids_by_key(
        &self,
        keys: &HashSet<&str>,
    ) -> anyhow::Result<HashMap<String, WorkId>> {
        let mut out = HashMap::new();
        let mut cursor = None;
        loop {
            let page = self
                .storage
                .list_works(&ListWorksQuery {
                    library: Some(self.library.id),
                    recursive: true,
                    sort: SortOrder::DateAddedAsc,
                    limit: 500,
                    cursor,
                    ..Default::default()
                })
                .await?;
            for work in &page.items {
                for ext in &work.external_ids {
                    if ext.kind == ExternalIdKind::Custom && keys.contains(ext.value.as_str()) {
                        out.insert(ext.value.clone(), work.id);
                    }
                }
            }
            cursor = page.next_cursor();
            if cursor.is_none() {
                return Ok(out);
            }
        }
    }

    async fn plan_movie_from(
        &mut self,
        classification: MovieClassification,
    ) -> anyhow::Result<PlannedTarget> {
        let scan_key = movie_scan_key(
            self.library.id,
            &classification.identity,
            &self.library_roots,
        );
        let (work_id, work_writes) = self
            .plan_work_node(
                &scan_key,
                WorkKind::Movie,
                title_or_fallback(&classification.parsed.title, "Unknown"),
                slugify(&classification.parsed.title),
                None,
                classification.parsed.year,
                None,
                None,
                None,
            )
            .await?;

        let edition_name = classification
            .parsed
            .edition
            .clone()
            .unwrap_or_else(|| "Default".to_string());
        let (edition_id, edition_write) = self.plan_edition(work_id, &edition_name).await?;

        let mut writes = work_writes;
        if let Some(edition) = edition_write {
            writes.push(WriteOp::Edition(edition));
        }
        Ok(PlannedTarget {
            work_id,
            edition_id,
            part_index: classification.parsed.part.unwrap_or(0),
            writes,
        })
    }

    async fn plan_music_from(&mut self, c: MusicClassification) -> anyhow::Result<PlannedTarget> {
        let keys = MusicKeys::new(self.library.id, &c, &self.library_roots);
        let (artist_id, artist_writes) = self
            .plan_work_node(
                &keys.artist,
                WorkKind::Artist,
                c.artist.clone(),
                slugify(&c.artist),
                None,
                None,
                None,
                None,
                None,
            )
            .await?;

        let (album_id, album_writes) = self
            .plan_work_node(
                &keys.album,
                WorkKind::Album,
                c.album.clone(),
                slugify(&c.album),
                Some(artist_id),
                None,
                None,
                None,
                None,
            )
            .await?;

        let (track_id, track_writes) = self
            .plan_work_node(
                &keys.track,
                WorkKind::Track,
                c.track.clone(),
                slugify(&c.track),
                Some(album_id),
                None,
                c.track_number.map(|n| n as i32),
                None,
                None,
            )
            .await?;
        let (edition_id, edition_write) = self.plan_edition(track_id, "Default").await?;

        let mut writes = artist_writes;
        writes.extend(album_writes);
        writes.extend(track_writes);
        if let Some(edition) = edition_write {
            writes.push(WriteOp::Edition(edition));
        }
        Ok(PlannedTarget {
            work_id: track_id,
            edition_id,
            part_index: 0,
            writes,
        })
    }

    async fn plan_photo_from(&mut self, c: PhotoClassification) -> anyhow::Result<PlannedTarget> {
        let key = format!(
            "fs:photo:{}:{}",
            self.library.id.into_uuid(),
            relative_slug(&c.path, &self.library_roots)
        );
        let (work_id, mut writes) = self
            .plan_work_node(
                &key,
                WorkKind::Photo,
                c.title.clone(),
                slugify(&c.title),
                None,
                c.year,
                None,
                None,
                None,
            )
            .await?;
        if let Some(release_date) = c.release_date {
            for write in &mut writes {
                if let WriteOp::Work(work) = write {
                    work.release_date = Some(release_date.clone());
                }
            }
        }
        let (edition_id, edition_write) = self.plan_edition(work_id, "Default").await?;
        if let Some(edition) = edition_write {
            writes.push(WriteOp::Edition(edition));
        }
        Ok(PlannedTarget {
            work_id,
            edition_id,
            part_index: 0,
            writes,
        })
    }

    async fn plan_episode_from(
        &mut self,
        c: EpisodeClassification,
    ) -> anyhow::Result<PlannedTarget> {
        let keys = EpisodeKeys::new(self.library.id, &c);
        let (series_id, series_writes) = self
            .plan_work_node(
                &keys.series,
                WorkKind::Series,
                c.parsed.show.clone(),
                slugify(&c.parsed.show),
                None,
                c.parsed.show_year,
                None,
                None,
                None,
            )
            .await?;

        let season_title = format!("Season {}", c.parsed.season);
        let season_sort = slugify(&season_title);
        let (season_id, season_writes) = self
            .plan_work_node(
                &keys.season,
                WorkKind::Season,
                season_title,
                season_sort,
                Some(series_id),
                None,
                Some(c.parsed.season as i32),
                None,
                None,
            )
            .await?;

        let episode_title = match (&c.parsed.title, c.parsed.episode_end) {
            (Some(t), _) => t.clone(),
            (None, Some(end)) => format!("Episodes {}–{end}", c.parsed.episode),
            (None, None) => format!("Episode {}", c.parsed.episode),
        };
        let episode_sort = slugify(&episode_title);
        let (episode_id, episode_writes) = self
            .plan_work_node(
                &keys.episode,
                WorkKind::Episode,
                episode_title,
                episode_sort,
                Some(season_id),
                None,
                Some(c.parsed.episode as i32),
                c.parsed.episode_end.map(|e| e as i32),
                Some(c.parsed.season as i32),
            )
            .await?;

        let (edition_id, edition_write) = self.plan_edition(episode_id, "Default").await?;

        let mut writes = Vec::new();
        writes.extend(series_writes);
        writes.extend(season_writes);
        writes.extend(episode_writes);
        if let Some(e) = edition_write {
            writes.push(WriteOp::Edition(e));
        }
        Ok(PlannedTarget {
            work_id: episode_id,
            edition_id,
            part_index: 0,
            writes,
        })
    }

    /// Returns `(work_id, writes)`. `writes` is empty when the work already
    /// exists and no write is needed.
    #[allow(clippy::too_many_arguments)]
    async fn plan_work_node(
        &self,
        key: &str,
        kind: WorkKind,
        title: String,
        sort_title: String,
        parent: Option<WorkId>,
        year: Option<i32>,
        index_number: Option<i32>,
        index_end: Option<i32>,
        parent_index: Option<i32>,
    ) -> anyhow::Result<(WorkId, Vec<WriteOp>)> {
        if let Some(existing) = self.find_work_by_key(key).await? {
            // Reuse an existing Work as-is. The scanner deliberately does NOT
            // touch its metadata_status — once a provider or a user has
            // escalated it past Unconfirmed, a rescan must not silently
            // downgrade it back to filesystem heuristics.
            return Ok((existing.id, Vec::new()));
        }
        let now = OffsetDateTime::now_utc();
        let work = Work {
            id: WorkId::new(),
            kind,
            parent_id: parent,
            library_id: self.library.id,
            title,
            sort_title,
            year,
            release_date: None,
            overview: None,
            external_ids: vec![ExternalId {
                kind: ExternalIdKind::Custom,
                value: key.to_string(),
            }],
            index_number,
            index_end,
            parent_index,
            // Filesystem-derived. A provider or a user has to escalate this.
            metadata_status: MetadataStatus::Unconfirmed,
            tagline: None,
            original_title: None,
            community_rating: None,
            locked_fields: vec![],
            added_at: now,
            updated_at: now,
        };
        let id = work.id;
        Ok((id, vec![WriteOp::Work(work)]))
    }

    async fn find_work_by_key(&self, key: &str) -> anyhow::Result<Option<Work>> {
        let mut hits = self
            .storage
            .works_by_external_id(ExternalIdKind::Custom, key)
            .await?;
        Ok(hits.pop())
    }

    /// Returns `(edition_id, write)`. `write` is `None` when the edition
    /// already exists and no write is needed.
    async fn plan_edition(
        &mut self,
        work_id: WorkId,
        name: &str,
    ) -> anyhow::Result<(EditionId, Option<Edition>)> {
        let cache_key = (work_id, name.to_string());
        if let Some(id) = self.edition_cache.get(&cache_key) {
            return Ok((*id, None));
        }
        let editions = self.storage.editions_for_work(work_id).await?;
        if let Some(found) = editions.iter().find(|e| e.name == name) {
            self.edition_cache.insert(cache_key, found.id);
            return Ok((found.id, None));
        }
        let edition = Edition {
            id: EditionId::new(),
            work_id,
            name: name.to_string(),
            runtime_ms: None,
            intro_start_ms: None,
            intro_end_ms: None,
            credits_start_ms: None,
            chapters: vec![],
            codec_video: None,
            codec_audio: None,
            width: None,
            height: None,
            bit_depth: None,
            hdr_format: None,
            audio_tracks: None,
            subtitle_tracks: None,
            video_codec: None,
            video_profile: None,
            video_level: None,
            pixel_format: None,
            color_primaries: None,
            transfer_characteristics: None,
            matrix_coefficients: None,
            dovi_profile: None,
            max_cll: None,
            max_fall: None,
            mastering_display_primary_r: None,
            mastering_display_primary_g: None,
            mastering_display_primary_b: None,
            mastering_display_white_point: None,
            mastering_display_min_luminance: None,
            mastering_display_max_luminance: None,
            frame_rate: None,
        };
        let id = edition.id;
        self.edition_cache.insert(cache_key, id);
        Ok((id, Some(edition)))
    }
}

/// Keep `edition_cache` pointing at rows that exist: after a commit,
/// replace planned IDs with the IDs `upsert_edition` actually used (a
/// concurrent writer may own the `(work, name)` row); after a rollback,
/// forget the plan's never-written editions.
fn reconcile_edition_cache(
    cache: &mut HashMap<(WorkId, String), EditionId>,
    planned: &[EditionId],
    committed: Option<&HashMap<EditionId, EditionId>>,
) {
    match committed {
        Some(aliases) => {
            for id in cache.values_mut() {
                if let Some(actual) = aliases.get(id) {
                    *id = *actual;
                }
            }
        }
        None => cache.retain(|_, id| !planned.contains(id)),
    }
}

struct PlannedTarget {
    /// The work the file lands under (the movie, or the episode for
    /// shows). The scan's `updated_works` event list is built from it.
    work_id: WorkId,
    edition_id: EditionId,
    part_index: u32,
    writes: Vec<WriteOp>,
}

struct TargetPlan {
    work_id: WorkId,
    writes: Vec<WriteOp>,
}

enum Classified {
    Movie(MovieClassification),
    Episode(EpisodeClassification),
    Music(MusicClassification),
    Photo(PhotoClassification),
}

enum WriteOp {
    Work(Work),
    Edition(Edition),
    MediaFile(MediaFile),
}

/// Overlay the v2 typed fields the [`ProbeReport`] carries onto
/// a freshly-created [`Edition`]. The legacy loose-string
/// `codec_video` / `codec_audio` / `hdr_format` fields are also
/// stamped so legacy importers keep working.
fn populate_edition_from_probe(mut edition: Edition, probe: &ProbeReport) -> Edition {
    if edition.runtime_ms.is_none() {
        edition.runtime_ms = probe.duration_ms.and_then(|ms| i64::try_from(ms).ok());
    }
    if let Some(v) = &probe.video {
        edition.video_codec = Some(v.codec);
        edition.video_profile = v.profile;
        edition.video_level = v.level;
        edition.pixel_format = v.pixel_format;
        edition.width = Some(v.width);
        edition.height = Some(v.height);
        edition.bit_depth = v.bit_depth.map(u32::from);
        edition.codec_video = Some(v.codec.as_str().to_string());
        edition.color_primaries = v.color_space;
        edition.transfer_characteristics = v.transfer;
        edition.matrix_coefficients = v.matrix;
        edition.dovi_profile = v.dovi_profile;
        edition.max_cll = v.side_data.max_cll;
        edition.max_fall = v.side_data.max_fall;
        edition.mastering_display_primary_r = v.side_data.mastering.as_ref().map(|m| m.primary_r);
        edition.mastering_display_primary_g = v.side_data.mastering.as_ref().map(|m| m.primary_g);
        edition.mastering_display_primary_b = v.side_data.mastering.as_ref().map(|m| m.primary_b);
        edition.mastering_display_white_point =
            v.side_data.mastering.as_ref().map(|m| m.white_point);
        edition.mastering_display_min_luminance =
            v.side_data.mastering.as_ref().map(|m| m.min_luminance);
        edition.mastering_display_max_luminance =
            v.side_data.mastering.as_ref().map(|m| m.max_luminance);
        edition.frame_rate = v.frame_rate;
        edition.hdr_format = v.transfer.map(|t| match t {
            HdrTransfer::Sdr => "SDR".to_string(),
            HdrTransfer::Pq => "HDR10".to_string(),
            HdrTransfer::Hlg => "HLG".to_string(),
            other => other.as_str().to_uppercase(),
        });
    }

    if !probe.audio.is_empty() {
        edition.audio_tracks = Some(
            probe
                .audio
                .iter()
                .map(|a| AudioTrack {
                    language: a.language.clone(),
                    codec: a.codec.clone(),
                    channels: a.channels,
                    default: a.default,
                    bit_depth: a.bit_depth,
                    sample_rate: a.sample_rate,
                    channel_layout: a.channel_layout,
                    codec_profile: a.profile.clone(),
                    audio_object_type: a.audio_object_type.clone(),
                    external: false,
                    forced: a.forced,
                    title: a.title.clone(),
                })
                .collect(),
        );
        if let Some(first) = probe.audio.first() {
            edition.codec_audio = Some(first.codec.clone());
        }
    }

    if !probe.subtitle.is_empty() {
        edition.subtitle_tracks = Some(
            probe
                .subtitle
                .iter()
                .map(|s| SubtitleTrack {
                    language: s.language.clone(),
                    codec: s.codec.as_str().to_string(),
                    default: s.default,
                    forced: s.forced,
                    external: s.external,
                    codec_profile: None,
                    srt_compatible: matches!(
                        s.codec,
                        SubtitleCodec::WebVtt
                            | SubtitleCodec::SubRip
                            | SubtitleCodec::Ttml
                            | SubtitleCodec::MovText
                    ),
                    image_count: None,
                    title: None,
                    path: None,
                    stream_index: None,
                })
                .collect(),
        );
    }

    edition
}

fn container_from_ext(path: &std::path::Path) -> Option<String> {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
}

/// Slugified relative path from the deepest library root that contains the
/// target. Falls back to the raw target if no root matches (shouldn't
/// happen in practice, but it keeps the function total).
fn relative_slug(target: &std::path::Path, roots: &[PathBuf]) -> String {
    let rel = roots
        .iter()
        .filter(|r| target.starts_with(r))
        .max_by_key(|r| r.as_os_str().len())
        .and_then(|r| target.strip_prefix(r).ok())
        .unwrap_or(target);
    slugify(&rel.to_string_lossy())
}

fn movie_scan_key(library_id: LibraryId, identity: &MovieIdentity, roots: &[PathBuf]) -> String {
    let lib_uuid = library_id.into_uuid();
    match identity {
        MovieIdentity::InFolder(p) => {
            format!("fs:movie:{lib_uuid}:folder:{}", relative_slug(p, roots))
        }
        MovieIdentity::Bare(p) => {
            // For bare files, drop the extension so two re-encodes of the
            // same bare file (`x.mkv` → `x.mp4`) still dedupe — but two
            // files with different stems remain separate.
            let stem_path = p.with_extension("");
            format!(
                "fs:movie:{lib_uuid}:bare:{}",
                relative_slug(&stem_path, roots)
            )
        }
    }
}

/// Scan keys of the series / season / episode works an episode file
/// lands under. None depends on the season folder, so every file of an
/// episode is a version of one work wherever it sits; a multi-episode
/// file keys its range and stays apart from the single episodes.
struct EpisodeKeys {
    series: String,
    season: String,
    episode: String,
}

impl EpisodeKeys {
    fn new(library_id: LibraryId, c: &EpisodeClassification) -> Self {
        let lib_uuid = library_id.into_uuid();
        let show_slug = slugify(&c.parsed.show);
        let season = c.parsed.season;
        let episode_range_slug = match c.parsed.episode_end {
            Some(end) => format!("{}-{end}", c.parsed.episode),
            None => format!("{}", c.parsed.episode),
        };
        Self {
            series: format!("fs:series:{lib_uuid}:{show_slug}"),
            season: format!("fs:season:{lib_uuid}:{show_slug}:{season}"),
            episode: format!("fs:episode:{lib_uuid}:{show_slug}:{season}:{episode_range_slug}"),
        }
    }
}

/// Scan keys of the artist / album / track works a music file lands under.
struct MusicKeys {
    artist: String,
    album: String,
    track: String,
}

impl MusicKeys {
    fn new(library_id: LibraryId, c: &MusicClassification, roots: &[PathBuf]) -> Self {
        let lib_uuid = library_id.into_uuid();
        Self {
            artist: format!(
                "fs:artist:{lib_uuid}:{}",
                relative_slug(&c.artist_path, roots)
            ),
            album: format!(
                "fs:album:{lib_uuid}:{}",
                relative_slug(&c.album_path, roots)
            ),
            track: format!(
                "fs:track:{lib_uuid}:{}",
                relative_slug(&c.track_path, roots)
            ),
        }
    }
}

fn title_or_fallback(title: &str, fallback: &str) -> String {
    if title.is_empty() {
        fallback.to_string()
    } else {
        title.to_string()
    }
}

/// Drain a work-id set into a sorted vec so the `ScanCompleted` wire
/// shape is deterministic (UUIDv7 sorts by creation time).
fn sorted_ids(ids: &HashSet<WorkId>) -> Vec<WorkId> {
    let mut v: Vec<WorkId> = ids.iter().copied().collect();
    v.sort_by_key(|id| id.into_uuid());
    v
}

pub(crate) async fn run(
    storage: &dyn Storage,
    library: LibraryId,
    events: Option<&DomainEventBus>,
) -> anyhow::Result<ScanSummary> {
    let lock = {
        let mut locks = SCAN_LOCKS.lock().await;
        locks
            .entry(library)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _scan_guard = lock.lock_owned().await;
    let run = ScanRun::new(storage, library, events).await?;
    run.run().await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use noflex_core::{EditionId, WorkId};

    use super::reconcile_edition_cache;

    #[test]
    fn committed_aliases_replace_planned_edition_ids() {
        let work = WorkId::new();
        let (planned, actual, other) = (EditionId::new(), EditionId::new(), EditionId::new());
        let mut cache = HashMap::from([
            ((work, "Theatrical".to_owned()), planned),
            ((work, "Director's Cut".to_owned()), other),
        ]);
        let aliases = HashMap::from([(planned, actual)]);
        reconcile_edition_cache(&mut cache, &[planned], Some(&aliases));
        assert_eq!(cache[&(work, "Theatrical".to_owned())], actual);
        assert_eq!(cache[&(work, "Director's Cut".to_owned())], other);
    }

    #[test]
    fn rollback_evicts_only_the_failed_plan() {
        let work = WorkId::new();
        let (planned, kept) = (EditionId::new(), EditionId::new());
        let mut cache = HashMap::from([
            ((work, "Theatrical".to_owned()), planned),
            ((work, "Extended".to_owned()), kept),
        ]);
        reconcile_edition_cache(&mut cache, &[planned], None);
        assert!(!cache.contains_key(&(work, "Theatrical".to_owned())));
        assert_eq!(cache[&(work, "Extended".to_owned())], kept);
    }
}
