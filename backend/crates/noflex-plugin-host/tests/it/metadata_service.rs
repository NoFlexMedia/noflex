// SPDX-License-Identifier: AGPL-3.0-only
//
// The host metadata service against real WASM fixtures: candidate order
// (library preferred providers vs. priority ranking), payload + artwork
// application with image-hook fallback and language preference, and
// automatic identification including series → season → episode chains.
//
// `tests/fixtures/artwork` answers deterministically (see its header);
// `tests/fixtures/concurrency` (the concurrency fixture) serves an image hook whose
// bytes are not an image, which exercises fallback to the next provider.
// Both are built on demand; the tests self-skip without the toolchain.

use std::sync::Arc;

use noflex_core::{
    ExternalId, ExternalIdKind, ImageKind, LibraryId, MetadataStatus, WorkField, WorkId, WorkKind,
    events::{DomainEvent, DomainEventBus},
};
use noflex_plugin_host::{
    AutoIdentify, EngineConfig, HookKind, MetadataService, MetadataWorker, Overrides, PluginId,
    PluginRegistry, PriorityTable, build_engine, spawn_metadata_worker,
};
use noflex_storage::{
    IndexingStage, Storage,
    domain::{Image, ImageSource, ImageTarget, Library, LibraryConfig, LibraryKind, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;
use tokio::{sync::RwLock, time::Duration};

const ART: &str = "artwork-fixture";
const CONCURRENCY: &str = "concurrency-fixture";

struct Harness {
    registry: Arc<PluginRegistry>,
    storage: Arc<dyn Storage>,
    image_dir: tempfile::TempDir,
}

impl Harness {
    /// `None` when the fixtures cannot be built here.
    async fn new(plugins: &[&str]) -> Option<Self> {
        let store = TursoStore::open_temp().await.expect("store");
        store.migrate().await.expect("migrate");
        let storage: Arc<dyn Storage> = Arc::new(store);
        let engine = build_engine(&EngineConfig::default()).expect("engine");
        let registry = Arc::new(
            PluginRegistry::new(engine)
                .expect("registry")
                .with_storage(storage.clone()),
        );
        for plugin in plugins {
            let wasm = match *plugin {
                ART => crate::fixtures::wasm("artwork")?,
                CONCURRENCY => crate::fixtures::wasm("concurrency")?,
                other => panic!("unknown fixture {other}"),
            };
            registry.load_from(&wasm).expect("load fixture");
        }
        Some(Self {
            registry,
            storage,
            image_dir: tempfile::tempdir().expect("image dir"),
        })
    }

    fn service(&self) -> MetadataService {
        MetadataService::new(self.registry.clone(), self.storage.clone())
            .with_image_dir(self.image_dir.path().to_path_buf())
    }

    fn priority_table(&self) -> PriorityTable {
        let plugins: Vec<_> = self
            .registry
            .snapshot()
            .into_iter()
            .map(|p| (PluginId(p.id), p.manifest))
            .collect();
        PriorityTable::build(plugins.iter().map(|(id, m)| (id, m)), &Overrides::default())
    }

    async fn library(&self, providers: &[&str], languages: &[&str]) -> Library {
        let library = Library {
            id: LibraryId::new(),
            name: "Films".into(),
            kind: LibraryKind::Movies,
            paths: vec!["/srv/films".into()],
            config: LibraryConfig {
                preferred_providers: providers.iter().map(|s| s.to_string()).collect(),
                preferred_languages: languages.iter().map(|s| s.to_string()).collect(),
                ..LibraryConfig::default()
            },
        };
        self.storage
            .upsert_library(&library)
            .await
            .expect("library");
        library
    }

    async fn work(&self, library: &Library, kind: WorkKind, title: &str) -> Work {
        let now = OffsetDateTime::now_utc();
        let work = Work {
            id: WorkId::new(),
            kind,
            parent_id: None,
            library_id: library.id,
            title: title.into(),
            sort_title: title.to_lowercase(),
            year: None,
            release_date: None,
            overview: None,
            tagline: None,
            original_title: None,
            community_rating: None,
            external_ids: vec![ExternalId {
                kind: ExternalIdKind::Custom,
                value: format!("fs:{title}"),
            }],
            index_number: None,
            index_end: None,
            parent_index: None,
            metadata_status: MetadataStatus::Unconfirmed,
            locked_fields: vec![],
            metadata_provider: None,
            added_at: now,
            updated_at: now,
        };
        self.storage.upsert_work(&work).await.expect("work");
        work
    }

    async fn save(&self, work: &Work) {
        self.storage.upsert_work(work).await.expect("save work");
    }

    async fn provider_art(&self, work: WorkId) -> Vec<(ImageKind, Option<String>)> {
        self.storage
            .list_images(ImageTarget::Work(work), &[ImageSource::Provider])
            .await
            .expect("images")
            .into_iter()
            .map(|i| {
                let bytes = std::fs::read(&i.path).expect("art file");
                assert!(bytes.starts_with(b"\x89PNG"), "stored bytes are the PNG");
                assert!(
                    std::path::Path::new(&i.path).starts_with(self.image_dir.path()),
                    "art lives under the image cache"
                );
                (i.kind, i.language)
            })
            .collect()
    }
}

fn tmdb(value: &str) -> ExternalId {
    ExternalId {
        kind: ExternalIdKind::Tmdb,
        value: value.into(),
    }
}

macro_rules! harness {
    ($($plugin:expr),*) => {
        match Harness::new(&[$($plugin),*]).await {
            Some(h) => h,
            None => {
                eprintln!("skipping: could not build the fixture plugins");
                return;
            }
        }
    };
}

#[tokio::test]
async fn candidates_follow_the_library_preferred_providers() {
    let h = harness!(ART, CONCURRENCY);
    let mut table = h.priority_table();
    table
        .set(CONCURRENCY, "metadata", "movie", 300)
        .expect("override");
    table.set(ART, "metadata", "movie", 100).expect("override");
    let service = h
        .service()
        .with_priority_table(Arc::new(RwLock::new(table)));
    let ids = |v: Vec<PluginId>| v.into_iter().map(|p| p.0).collect::<Vec<_>>();

    let preferred = h.library(&[ART, "ghost", CONCURRENCY, ART], &[]).await;
    assert_eq!(
        ids(service
            .candidates(HookKind::Metadata, WorkKind::Movie, Some(&preferred))
            .await),
        vec![ART, CONCURRENCY],
        "preferred order wins over priority; unloaded and repeated ids drop out"
    );
    assert_eq!(
        ids(service
            .candidates(HookKind::Metadata, WorkKind::Track, Some(&preferred))
            .await),
        vec![CONCURRENCY],
        "a plugin that does not support the kind is filtered"
    );
    assert_eq!(
        ids(service
            .candidates(HookKind::Image, WorkKind::Movie, Some(&preferred))
            .await),
        vec![ART, CONCURRENCY]
    );

    let unset = h.library(&[], &[]).await;
    assert_eq!(
        ids(service
            .candidates(HookKind::Metadata, WorkKind::Movie, Some(&unset))
            .await),
        vec![CONCURRENCY, ART],
        "no preference keeps the priority ranking"
    );
}

#[tokio::test]
async fn refresh_applies_the_payload_and_stores_provider_art() {
    let h = harness!(ART);
    let library = h.library(&[ART], &["de"]).await;
    let mut work = h.work(&library, WorkKind::Movie, "Heat").await;
    work.external_ids.push(tmdb("movie/heat"));
    work.locked_fields = vec![WorkField::Title];
    h.save(&work).await;
    let target = ImageTarget::Work(work.id);
    for (kind, source, path) in [
        (
            ImageKind::Primary,
            ImageSource::Local,
            "/srv/films/Heat/poster.jpg",
        ),
        (
            ImageKind::Logo,
            ImageSource::Provider,
            "/old/provider/logo.png",
        ),
    ] {
        h.storage
            .upsert_image(&Image {
                target,
                kind,
                source,
                path: path.into(),
                width: None,
                height: None,
                language: None,
                provider: None,
            })
            .await
            .expect("seed image");
    }

    let refreshed = h.service().refresh(work.id, false).await.expect("refresh");
    assert_eq!(refreshed.title, "Heat", "locked title survives");
    assert_eq!(refreshed.overview.as_deref(), Some("from fixture"));
    assert_eq!(refreshed.metadata_status, MetadataStatus::Confirmed);
    assert_eq!(
        h.provider_art(work.id).await,
        vec![
            (ImageKind::Backdrop, None),
            (ImageKind::Primary, Some("de".to_string())),
        ],
        "payload kinds fetched in the library language; the stale logo is gone"
    );
    let local = h
        .storage
        .image(target, ImageKind::Primary, &[ImageSource::Local])
        .await
        .expect("local");
    assert!(local.is_some(), "local art is never touched");
    let tags = h
        .storage
        .tag_assignments_for_target(noflex_core::TagTarget::Work(work.id))
        .await
        .expect("tags");
    assert!(
        tags.iter()
            .any(|t| t.tag.display == "Drama" && t.source == "provider:artwork-fixture")
    );

    // Refreshing again replaces the provider files in place.
    h.service().refresh(work.id, false).await.expect("refresh");
    assert_eq!(h.provider_art(work.id).await.len(), 2);
}

#[tokio::test]
async fn picked_artwork_is_kept_through_refreshes() {
    let h = harness!(CONCURRENCY, ART);
    let library = h.library(&[ART], &[]).await;
    let mut work = h.work(&library, WorkKind::Movie, "Heat").await;
    work.external_ids.push(tmdb("movie/heat"));
    h.save(&work).await;
    let service = h.service();

    let candidates = service
        .artwork_candidates(work.id, ImageKind::Primary)
        .await
        .expect("candidates");
    assert_eq!(
        candidates
            .iter()
            .map(|c| (c.provider.as_str(), c.score))
            .collect::<Vec<_>>(),
        vec![(ART, Some(1.0)), (ART, Some(0.5))],
        "every provider asked, preferred first, each in its own order"
    );
    let preview = service
        .candidate_preview(&candidates[1].id[..])
        .await
        .expect("preview");
    assert_eq!(preview.mime, "image/png");

    let other = h.work(&library, WorkKind::Movie, "Ronin").await;
    assert!(matches!(
        service.pick_artwork(other.id, &candidates[1].id[..]).await,
        Err(noflex_plugin_host::MetadataError::UnknownCandidate)
    ));
    assert!(matches!(
        service.candidate_preview("not-offered").await,
        Err(noflex_plugin_host::MetadataError::UnknownCandidate)
    ));
    assert!(matches!(
        service.artwork_candidates(work.id, ImageKind::Disc).await,
        Err(noflex_plugin_host::MetadataError::UnpickableKind(_))
    ));

    let picked = service
        .pick_artwork(work.id, &candidates[1].id[..])
        .await
        .expect("pick");
    assert!(picked.locked_fields.contains(&WorkField::PrimaryImage));
    let target = ImageTarget::Work(work.id);
    let poster = || async {
        h.storage
            .image(target, ImageKind::Primary, &[ImageSource::Provider])
            .await
            .expect("image")
            .expect("picked poster stored")
    };
    let stored = poster().await;
    assert_eq!(
        stored.width,
        Some(2),
        "the full-size image, not the preview"
    );
    assert_eq!(
        stored.provider.as_deref(),
        Some(ART),
        "credited to its provider"
    );

    service.refresh(work.id, false).await.expect("refresh");
    assert_eq!(poster().await.path, stored.path, "a refresh keeps the pick");
    assert!(
        h.provider_art(work.id)
            .await
            .contains(&(ImageKind::Backdrop, None)),
        "other kinds still refresh"
    );
}

#[tokio::test]
async fn credited_people_get_photos_from_the_crediting_provider() {
    let h = harness!(ART);
    let library = h.library(&[ART], &[]).await;
    let mut work = h.work(&library, WorkKind::Movie, "Heat").await;
    // The scanner's own key would answer first, without anyone credited.
    work.external_ids = vec![tmdb("movie/cast")];
    h.save(&work).await;

    h.service().refresh(work.id, false).await.expect("refresh");
    let people: Vec<_> = h
        .storage
        .tag_assignments_for_target(noflex_core::TagTarget::Work(work.id))
        .await
        .expect("tags")
        .into_iter()
        .filter(|a| a.tag.namespace == "person")
        .map(|a| a.tag)
        .collect();
    assert_eq!(people.len(), 2);
    let photo = |person: &noflex_storage::domain::Tag| {
        let storage = h.storage.clone();
        let id = person.id;
        async move {
            storage
                .image(
                    ImageTarget::Tag(id),
                    ImageKind::Primary,
                    &[ImageSource::Provider],
                )
                .await
                .expect("image")
        }
    };
    let (with, without) = if people[0].display == "Person 1" {
        (&people[0], &people[1])
    } else {
        (&people[1], &people[0])
    };
    assert_eq!(
        noflex_metadata::person_ids(&with.metadata.0),
        vec![tmdb("person/1")],
        "the tag keeps the provider's ids for the person"
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let stored = loop {
        if let Some(image) = photo(with).await {
            break image;
        }
        assert!(std::time::Instant::now() < deadline, "no photo arrived");
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(stored.provider.as_deref(), Some(ART));
    assert!(
        std::path::Path::new(&stored.path).starts_with(h.image_dir.path()),
        "the photo lives under the image cache"
    );
    assert!(
        photo(without).await.is_none(),
        "a person it has no photo of"
    );
}

#[tokio::test]
async fn a_loaded_plugin_says_how_to_credit_it() {
    let h = harness!(ART, CONCURRENCY);
    let credit = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let rows = h.storage.list_provider_attributions().await.expect("list");
            if let Some(row) = rows.into_iter().find(|row| row.provider == ART) {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the fixture's credit is stored after it loads");
    assert_eq!(credit.name, "Artwork Test Fixture");
    assert_eq!(credit.text, "Data from the fixture.");
    assert_eq!(credit.url.as_deref(), Some("https://fixture.example"));
    assert_eq!(credit.logo_mime.as_deref(), Some("image/png"));
    let logo = h
        .storage
        .provider_attribution_logo(ART)
        .await
        .expect("logo")
        .expect("the fixture has a logo");
    assert!(logo.bytes.starts_with(b"\x89PNG"));

    let rows = h.storage.list_provider_attributions().await.expect("list");
    assert!(
        !rows.iter().any(|row| row.provider == CONCURRENCY),
        "the concurrency fixture asks for no credit"
    );
}

#[tokio::test]
async fn artwork_falls_back_to_the_next_image_provider() {
    let h = harness!(ART, CONCURRENCY);
    // The concurrency fixture wins the metadata fetch, but its image
    // bytes are not an image.
    let library = h.library(&[CONCURRENCY, ART], &[]).await;
    let mut work = h.work(&library, WorkKind::Movie, "Fallback").await;
    work.external_ids.push(tmdb("movie/fallback"));
    h.save(&work).await;

    let refreshed = h.service().refresh(work.id, false).await.expect("refresh");
    assert_eq!(refreshed.title, "Fixture Title", "metadata from the winner");
    assert_eq!(
        h.provider_art(work.id).await,
        vec![(ImageKind::Primary, None)],
        "the winner's payload lists only a poster, fetched from the fallback"
    );
    assert_eq!(refreshed.metadata_provider.as_deref(), Some(CONCURRENCY));
    let poster = h
        .storage
        .image(
            ImageTarget::Work(work.id),
            ImageKind::Primary,
            &[ImageSource::Provider],
        )
        .await
        .expect("image")
        .expect("poster");
    assert_eq!(
        poster.provider.as_deref(),
        Some(ART),
        "art is credited to whoever supplied it, not to the metadata's provider"
    );
}

#[tokio::test]
async fn image_language_falls_through_the_preferred_list() {
    let h = harness!(ART);
    let library = h.library(&[ART], &["xx", "de"]).await;
    let mut work = h.work(&library, WorkKind::Movie, "Polyglot").await;
    work.external_ids.push(tmdb("movie/polyglot"));
    work.metadata_status = MetadataStatus::Confirmed;
    h.save(&work).await;

    let written = h.service().backfill_artwork(&work).await.expect("backfill");
    assert_eq!(written, 2, "poster and backdrop; the fixture has no logo");
    assert_eq!(
        h.provider_art(work.id).await,
        vec![
            (ImageKind::Backdrop, None),
            (ImageKind::Primary, Some("de".to_string())),
        ],
        "`xx` came back in English, so `de` was asked next"
    );
}

#[tokio::test]
async fn auto_identify_takes_only_confident_matches_of_unconfirmed_works() {
    let h = harness!(ART);
    let library = h.library(&[ART], &[]).await;
    let service = h.service();

    let heat = h.work(&library, WorkKind::Movie, "Heat").await;
    let AutoIdentify::Matched { provider, work } =
        service.auto_identify(heat.id).await.expect("identify")
    else {
        panic!("Heat should match");
    };
    assert_eq!(provider.0, ART);
    assert_eq!(work.metadata_status, MetadataStatus::Confirmed);
    assert!(work.external_ids.contains(&tmdb("movie/heat")));
    assert!(
        work.external_ids
            .iter()
            .any(|i| i.kind == ExternalIdKind::Custom),
        "the scan key is kept"
    );
    assert_eq!(h.provider_art(heat.id).await.len(), 2);

    for title in ["Ambiguous", "Weak"] {
        let w = h.work(&library, WorkKind::Movie, title).await;
        assert!(
            matches!(
                service.auto_identify(w.id).await.expect("identify"),
                AutoIdentify::NoMatch
            ),
            "{title}"
        );
        let stored = h.storage.get_work(w.id).await.unwrap().unwrap();
        assert_eq!(stored.metadata_status, MetadataStatus::Unconfirmed);
    }

    let mut edited = h.work(&library, WorkKind::Movie, "Edited").await;
    edited.metadata_status = MetadataStatus::UserEdited;
    h.save(&edited).await;
    let mut confirmed = h.work(&library, WorkKind::Movie, "Confirmed").await;
    confirmed.metadata_status = MetadataStatus::Confirmed;
    h.save(&confirmed).await;
    let mut has_id = h.work(&library, WorkKind::Movie, "Known").await;
    has_id.external_ids.push(tmdb("movie/elsewhere"));
    h.save(&has_id).await;
    let track = h.work(&library, WorkKind::Track, "A Song").await;
    for w in [&edited, &confirmed, &has_id, &track] {
        assert!(
            matches!(
                service.auto_identify(w.id).await.expect("identify"),
                AutoIdentify::Skipped
            ),
            "{} is not eligible",
            w.title
        );
        let stored = h.storage.get_work(w.id).await.unwrap().unwrap();
        assert_eq!(stored.title, w.title, "untouched");
    }
}

#[tokio::test]
async fn seasons_and_episodes_match_through_their_identified_series() {
    let h = harness!(ART);
    let library = h.library(&[ART], &[]).await;
    let service = h.service();
    let show = h.work(&library, WorkKind::Series, "The Show").await;
    let mut season = h.work(&library, WorkKind::Season, "Season 1").await;
    season.parent_id = Some(show.id);
    season.index_number = Some(1);
    h.save(&season).await;
    let mut episode = h.work(&library, WorkKind::Episode, "Pilot").await;
    episode.parent_id = Some(season.id);
    episode.index_number = Some(2);
    episode.parent_index = Some(1);
    h.save(&episode).await;

    assert!(
        matches!(
            service.auto_identify(episode.id).await.expect("episode"),
            AutoIdentify::Skipped
        ),
        "the series is not identified yet"
    );
    for id in [show.id, season.id, episode.id] {
        assert!(matches!(
            service.auto_identify(id).await.expect("identify"),
            AutoIdentify::Matched { .. }
        ));
    }
    let ids = |w: Work| {
        w.external_ids
            .into_iter()
            .filter(|i| i.kind == ExternalIdKind::Tmdb)
            .map(|i| i.value)
            .collect::<Vec<_>>()
    };
    let get = |id| {
        let storage = h.storage.clone();
        async move { storage.get_work(id).await.unwrap().unwrap() }
    };
    assert_eq!(ids(get(show.id).await), vec!["tv/the-show"]);
    assert_eq!(ids(get(season.id).await), vec!["tv/the-show/season/1"]);
    assert_eq!(
        ids(get(episode.id).await),
        vec!["tv/the-show/season/1/episode/2"]
    );
}

#[tokio::test]
async fn worker_sweep_identifies_parents_first_then_backfills_art() {
    let h = harness!(ART);
    let library = h.library(&[ART], &[]).await;
    let show = h.work(&library, WorkKind::Series, "Sweep Show").await;
    let mut season = h.work(&library, WorkKind::Season, "Season 3").await;
    season.parent_id = Some(show.id);
    season.index_number = Some(3);
    h.save(&season).await;
    let movie = h.work(&library, WorkKind::Movie, "Sweep Movie").await;
    let mut known = h.work(&library, WorkKind::Movie, "Known").await;
    known.external_ids.push(tmdb("movie/known"));
    known.metadata_status = MetadataStatus::Confirmed;
    h.save(&known).await;
    let mut artless = h.work(&library, WorkKind::Movie, "Artless").await;
    artless.external_ids.push(tmdb("movie/noart"));
    artless.metadata_status = MetadataStatus::Confirmed;
    h.save(&artless).await;
    let _ambiguous = h.work(&library, WorkKind::Movie, "Ambiguous").await;

    let worker = MetadataWorker::new(Arc::new(h.service()));
    let first = worker.run_sweep().await;
    assert_eq!(first.identified, 3, "series, its season, and the movie");
    assert_eq!(first.unmatched, 1);
    assert_eq!(
        first.artwork_written, 2,
        "the known movie's poster and backdrop"
    );
    for id in [show.id, season.id, movie.id, known.id] {
        assert!(!h.provider_art(id).await.is_empty());
    }
    assert!(h.provider_art(artless.id).await.is_empty());

    let second = worker.run_sweep().await;
    assert_eq!(
        second,
        Default::default(),
        "matched works are done; misses are parked"
    );
}

#[tokio::test]
async fn indexing_follows_a_scan_into_the_sweep_and_ends_with_its_answer() {
    let h = harness!(ART);
    let library = h.library(&[ART], &[]).await;
    let service = Arc::new(h.service());
    let indexing = service.indexing().clone();
    let bus = DomainEventBus::new(8);
    let worker = spawn_metadata_worker(service, &bus);
    // The sweep at startup has nothing to do yet, and has to be out of the way.
    tokio::time::sleep(Duration::from_millis(50)).await;

    bus.emit(DomainEvent::ScanStarted {
        library: library.id,
        library_name: library.name.clone(),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let matched = h.work(&library, WorkKind::Movie, "Sweep Movie").await;
    let unmatched = h.work(&library, WorkKind::Movie, "Ambiguous").await;
    assert_eq!(indexing.stage(&matched), Some(IndexingStage::Adding));

    bus.emit(DomainEvent::ScanCompleted {
        library: library.id,
        library_name: library.name.clone(),
        added: 2,
        updated: 0,
        removed: 0,
        added_works: vec![matched.id, unmatched.id],
        updated_works: vec![],
        removed_works: vec![],
    });
    let settled = async {
        loop {
            let fresh = h
                .storage
                .get_work(matched.id)
                .await
                .expect("get")
                .expect("work");
            if indexing.stage(&fresh).is_none() && indexing.stage(&unmatched).is_none() {
                break fresh;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let fresh = tokio::time::timeout(Duration::from_secs(10), settled)
        .await
        .expect("indexing ends");
    assert_eq!(fresh.metadata_status, MetadataStatus::Confirmed);
    worker.abort();
}
