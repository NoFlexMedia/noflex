// SPDX-License-Identifier: AGPL-3.0-only
//
// The library write imports (`host-tags`, `host-collections`,
// `host-user-state`) and the `host-library` lookups, driven through the
// real wasmtime dispatch by the fixture guest in
// `tests/fixtures/library-writes/`, whose `link-complete` runs one command
// as its user and reports the outcome. Self-skips when the fixture can't be
// built.

use std::{sync::Arc, time::Duration};

use noflex_core::{
    DomainEvent, DomainEventBus, ExternalId, ExternalIdKind, LibraryId, PlayTarget, TagTarget,
    UserId, WorkId, WorkKind,
};
use noflex_plugin_host::{EngineConfig, LoadedPlugin, PluginId, PluginRegistry, build_engine};
use noflex_storage::{
    Storage,
    domain::{Library, User, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

const PLUGIN_ID: &str = "fixture-library-writes";

struct Harness {
    storage: Arc<dyn Storage>,
    events: DomainEventBus,
    plugin: Arc<LoadedPlugin>,
    library: LibraryId,
    _registry: PluginRegistry,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn start() -> Option<Self> {
        let Some(wasm) = crate::fixtures::wasm("library-writes") else {
            eprintln!("skipping: could not build the fixture-library-writes plugin");
            return None;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TursoStore::open(dir.path().join("writes.db"))
            .await
            .expect("open store");
        store.migrate().await.expect("migrate");
        let storage: Arc<dyn Storage> = Arc::new(store);
        let library = Library {
            id: LibraryId::new(),
            name: "Films".into(),
            kind: noflex_storage::domain::LibraryKind::Movies,
            paths: vec!["/srv/films".into()],
            ..Default::default()
        };
        storage.upsert_library(&library).await.expect("library");
        let events = DomainEventBus::new(64);
        let engine = build_engine(&EngineConfig::default()).expect("engine");
        let registry = PluginRegistry::new(engine)
            .expect("registry")
            .with_storage(Arc::clone(&storage))
            .with_event_bus(events.clone());
        registry.load_from(&wasm).expect("load fixture");
        let plugin = registry
            .get_plugin(&PluginId(PLUGIN_ID.into()))
            .expect("fixture registered");
        Some(Self {
            storage,
            events,
            plugin,
            library: library.id,
            _registry: registry,
            _dir: dir,
        })
    }

    async fn user(&self, name: &str) -> UserId {
        let id = UserId::new();
        self.storage
            .upsert_user(&User {
                id,
                username: name.into(),
                display_name: None,
                password_hash: String::new(),
                is_admin: false,
                role: noflex_core::UserRole::User,
                created_at: OffsetDateTime::now_utc(),
                avatar_updated_at: None,
                has_pin: false,
            })
            .await
            .expect("user");
        self.storage
            .grant_library_access(id, self.library)
            .await
            .expect("grant");
        id
    }

    async fn work(&self, kind: WorkKind, title: &str, parent: Option<&Work>) -> Work {
        let now = OffsetDateTime::now_utc();
        let work = Work {
            id: WorkId::new(),
            kind,
            parent_id: parent.map(|parent| parent.id),
            library_id: self.library,
            title: title.into(),
            sort_title: title.to_lowercase(),
            year: None,
            release_date: None,
            overview: None,
            external_ids: vec![],
            index_number: None,
            index_end: None,
            parent_index: None,
            metadata_status: noflex_core::MetadataStatus::Unconfirmed,
            tagline: None,
            original_title: None,
            community_rating: None,
            locked_fields: vec![],
            metadata_provider: None,
            added_at: now,
            updated_at: now,
        };
        self.storage.upsert_work(&work).await.expect("work");
        work
    }

    /// Run one fixture command as `user`.
    async fn run(&self, user: UserId, command: &str) -> String {
        let secrets = self
            .plugin
            .call_link_complete(user.to_string(), "fixture".into(), command.into())
            .await
            .expect("dispatch")
            .expect("command runs");
        secrets[0].value.clone()
    }
}

#[tokio::test]
async fn lookups_find_works_by_provider_id_and_episodes_by_number() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let user = h.user("lookup").await;
    let mut film = h.work(WorkKind::Movie, "The Matrix", None).await;
    film.external_ids = vec![ExternalId {
        kind: ExternalIdKind::Tmdb,
        value: "movie/603".into(),
    }];
    h.storage.upsert_work(&film).await.expect("ids");
    let mut show = h.work(WorkKind::Series, "Show", None).await;
    show.external_ids = vec![ExternalId {
        kind: ExternalIdKind::Tvdb,
        value: "series/81189".into(),
    }];
    h.storage.upsert_work(&show).await.expect("ids");
    let season = h.work(WorkKind::Season, "Season 2", Some(&show)).await;
    let mut episode = h.work(WorkKind::Episode, "S02E05", Some(&season)).await;
    episode.parent_index = Some(2);
    episode.index_number = Some(5);
    h.storage.upsert_work(&episode).await.expect("numbers");

    assert_eq!(
        h.run(user, "find:tmdb:movie/603").await,
        film.id.to_string()
    );
    assert_eq!(
        h.run(user, "find:tmdb:603").await,
        "",
        "values match as stored"
    );
    assert_eq!(
        h.run(user, "episode:tvdb:series/81189:2:5").await,
        episode.id.to_string()
    );
    assert_eq!(h.run(user, "episode:tvdb:series/81189:2:6").await, "none");
}

#[tokio::test]
async fn tags_go_only_to_claimed_namespaces_and_carry_the_plugin() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let user = h.user("tagger").await;
    let film = h.work(WorkKind::Movie, "Tagged", None).await;
    let mut events = h.events.subscribe();

    let command = format!("tags:{}:fixture-list:watchlist,trending", film.id);
    assert_eq!(h.run(user, &command).await, "ok");
    assert_eq!(h.run(user, &command).await, "ok", "a retry is harmless");
    let assignments = h
        .storage
        .tag_assignments_for_target(TagTarget::Work(film.id))
        .await
        .expect("assignments");
    assert_eq!(assignments.len(), 2);
    assert!(
        assignments
            .iter()
            .all(|a| a.source == "plugin:fixture-library-writes")
    );
    assert!(matches!(
        events.recv().await.expect("event"),
        DomainEvent::PluginTagsSet { work, .. } if work == film.id
    ));

    assert_eq!(
        h.run(user, &format!("tags:{}:genre:drama", film.id)).await,
        "err:capability-denied",
        "a namespace the manifest doesn't claim"
    );
    assert_eq!(
        h.run(user, &format!("tags:{}:fixture-list:Bad Slug", film.id))
            .await,
        "err:bad-input"
    );
    assert_eq!(
        h.run(user, &format!("tags:{}:fixture-list:x", WorkId::new()))
            .await,
        "err:not-found"
    );
}

#[tokio::test]
async fn collections_sync_under_the_plugin() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let user = h.user("curator").await;
    let first = h.work(WorkKind::Movie, "First", None).await;
    let second = h.work(WorkKind::Movie, "Second", None).await;

    let command = format!("collection:picks:{},{}", second.id, first.id);
    assert_eq!(h.run(user, &command).await, "ok:1");
    let collections = h
        .storage
        .list_collections(h.library, 10, None)
        .await
        .expect("collections")
        .items;
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].plugin_id.as_deref(), Some(PLUGIN_ID));
    assert_eq!(collections[0].title, "List picks");
    assert_eq!(h.run(user, "collection:picks:").await, "ok:0");
    assert!(
        h.storage
            .list_collections(h.library, 10, None)
            .await
            .expect("collections")
            .items
            .is_empty(),
        "an empty list deletes the key's collections"
    );
}

#[tokio::test]
async fn user_state_is_bound_to_its_user_and_never_read_back() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let user = h.user("viewer").await;
    let other = h.user("someone-else").await;
    let imported = h.work(WorkKind::Movie, "Seen Elsewhere", None).await;
    let local = h.work(WorkKind::Movie, "Seen Here", None).await;
    let mut events = h.events.subscribe();
    let played = 1_700_000_000_000_i64;

    let import = |user: UserId| format!("import:{user}:{}:{played}:8:{played}", imported.id);
    assert_eq!(
        h.run(user, &import(other)).await,
        "err:capability-denied",
        "only the user the call acts for"
    );
    assert_eq!(h.run(user, &import(user)).await, "ok:1");
    assert_eq!(
        h.run(user, &import(user)).await,
        "ok:0",
        "a repeat changes nothing"
    );
    match events.recv().await.expect("event") {
        DomainEvent::UserStateImported {
            user: who, works, ..
        } => {
            assert_eq!(who, user);
            assert_eq!(works, [imported.id]);
        }
        other => panic!("unexpected event {other:?}"),
    }
    let data = h
        .storage
        .user_data(user, PlayTarget::Work(imported.id))
        .await
        .expect("data");
    assert!(data.played);
    assert_eq!(data.rating, Some(8.0));

    assert_eq!(
        h.run(user, &format!("history:{user}:-")).await,
        "",
        "the plugin's own imports aren't read back"
    );
    h.storage
        .mark_played(user, PlayTarget::Work(local.id), true)
        .await
        .expect("local play");
    let history = h.run(user, &format!("history:{user}:-")).await;
    assert!(
        history.starts_with(&format!("{}=", local.id)) && history.ends_with("/-"),
        "a local play is: {history}"
    );
    assert_eq!(
        h.run(user, &format!("history:{other}:-")).await,
        "err:capability-denied"
    );
}

#[tokio::test]
async fn per_user_tasks_run_for_each_linked_user() {
    let Some(h) = Harness::start().await else {
        return;
    };
    let linked = h.user("linked").await;
    let unlinked = h.user("unlinked").await;
    h.storage
        .set_user_plugin_secret(linked, PLUGIN_ID, "ticked", "no")
        .await
        .expect("link");

    let ticked = |user| {
        let storage = Arc::clone(&h.storage);
        async move {
            storage
                .user_plugin_secret(user, PLUGIN_ID, "ticked")
                .await
                .expect("read")
                .map(|secret| secret.value)
        }
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while ticked(linked).await.as_deref() != Some("yes") {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the linked user's run never came"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(ticked(unlinked).await, None, "nobody else is run for");
}
