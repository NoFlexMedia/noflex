// SPDX-License-Identifier: AGPL-3.0-only
//
// Downloaded subtitles through GraphQL: they follow the file's own tracks
// in `subtitleTracks`, marked with where they came from; anyone who can
// play the title searches and downloads, and only whoever downloaded one,
// or an administrator, removes it. The provider side is covered by the
// plugin host's tests; with no subtitle plugin loaded a search finds
// nothing and a download has nothing on offer.

use std::sync::Arc;

use async_graphql::Request;
use noflex_core::{
    DownloadedSubtitleId, EditionId, LibraryId, MediaFileId, SubtitleCodec, UserId, UserRole,
    WorkId, WorkKind,
};
use noflex_graphql::{NoflexSchema, SchemaContext, build_schema};
use noflex_plugin_host::{EngineConfig, PluginRegistry, SubtitleService, build_engine};
use noflex_storage::{
    RequestUser,
    domain::{
        DownloadedSubtitle, Edition, Library, LibraryConfig, LibraryKind, MediaFile, SubtitleTrack,
        User, Work,
    },
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use serde_json::Value;
use time::OffsetDateTime;

struct Fixture {
    schema: NoflexSchema,
    store: Arc<TursoStore>,
    media: MediaFileId,
    edition: EditionId,
    admin: RequestUser,
    viewer: RequestUser,
    other: RequestUser,
    outsider: RequestUser,
    _cache: tempfile::TempDir,
}

async fn user(store: &TursoStore, name: &str, is_admin: bool) -> RequestUser {
    let user = User {
        id: UserId::new(),
        username: name.into(),
        display_name: None,
        password_hash: String::new(),
        is_admin,
        role: if is_admin {
            UserRole::Admin
        } else {
            UserRole::User
        },
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
        has_pin: false,
    };
    store.upsert_user(&user).await.expect("user");
    RequestUser {
        user_id: user.id,
        is_admin,
    }
}

async fn fixture() -> Fixture {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let admin = user(&store, "admin", true).await;
    let viewer = user(&store, "viewer", false).await;
    let other = user(&store, "other", false).await;
    let outsider = user(&store, "outsider", false).await;

    let library = Library {
        id: LibraryId::new(),
        name: "Films".into(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/films".into()],
        config: LibraryConfig::default(),
    };
    store.upsert_library(&library).await.expect("library");
    for user in [viewer, other] {
        store
            .grant_library_access(user.user_id, library.id)
            .await
            .expect("grant");
    }
    let now = OffsetDateTime::now_utc();
    let work = Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Alien".into(),
        sort_title: "alien".into(),
        year: Some(1979),
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
    store.upsert_work(&work).await.expect("work");
    let edition = Edition {
        id: EditionId::new(),
        work_id: work.id,
        name: "Default".into(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    store.upsert_edition(&edition).await.expect("edition");
    let media = MediaFile {
        id: MediaFileId::new(),
        edition_id: edition.id,
        path: "/srv/films/Alien.mkv".into(),
        part_index: 0,
        size_bytes: 1024,
        mtime: now,
        container: Some("matroska".into()),
        probe: serde_json::json!({}).into(),
        subtitle_tracks: vec![SubtitleTrack {
            language: Some("eng".into()),
            codec: "sub_rip".into(),
            default: false,
            forced: false,
            external: true,
            codec_profile: None,
            srt_compatible: true,
            image_count: None,
            title: None,
            path: Some("/srv/films/Alien.en.srt".into()),
            stream_index: None,
            downloaded: None,
        }],
    };
    store.upsert_media_file(&media).await.expect("media");

    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(PluginRegistry::new(engine).expect("registry"));
    let cache = tempfile::tempdir().expect("cache");
    let storage: Arc<dyn Storage> = store.clone();
    let subtitles = Arc::new(SubtitleService::new(
        registry.clone(),
        storage,
        cache.path().join("subtitles"),
    ));
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        plugins: Some(registry),
        subtitles: Some(subtitles),
        ..Default::default()
    });
    Fixture {
        schema,
        store,
        media: media.id,
        edition: edition.id,
        admin,
        viewer,
        other,
        outsider,
        _cache: cache,
    }
}

async fn downloaded(f: &Fixture, by: RequestUser, handle: &str) -> DownloadedSubtitle {
    let id = DownloadedSubtitleId::new();
    let now = OffsetDateTime::now_utc();
    f.store
        .insert_downloaded_subtitle(&DownloadedSubtitle {
            id,
            media_file_id: f.media,
            provider: "opensubtitles".into(),
            handle: handle.into(),
            language: "fra".into(),
            format: SubtitleCodec::SubRip,
            release: Some("Alien.1979.1080p".into()),
            hearing_impaired: true,
            forced: false,
            hash_match: true,
            file_name: format!("{id}.srt"),
            downloaded_by: Some(by.user_id),
            created_at: now,
            last_used_at: now,
        })
        .await
        .expect("insert")
}

async fn run(f: &Fixture, as_user: RequestUser, query: String) -> async_graphql::Response {
    f.schema.execute(Request::new(query).data(as_user)).await
}

fn code(resp: &async_graphql::Response) -> Option<String> {
    let error = resp.errors.first()?;
    let extensions = serde_json::to_value(error.extensions.as_ref()?).ok()?;
    extensions["code"].as_str().map(str::to_string)
}

#[tokio::test]
async fn downloaded_subtitles_follow_the_files_own_tracks() {
    let f = fixture().await;
    let subtitle = downloaded(&f, f.viewer, "file-1").await;
    let resp = run(
        &f,
        f.other,
        format!(
            r#"{{ mediaFiles(editionId: "{}") {{ subtitleTracks {{ language codec title external path
                downloaded {{ id provider language hashMatch downloadedBy }} }} }} }}"#,
            f.edition
        ),
    )
    .await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    let tracks = resp.data.into_json().unwrap()["mediaFiles"][0]["subtitleTracks"].clone();
    assert_eq!(tracks.as_array().map(Vec::len), Some(2), "{tracks}");
    assert_eq!(tracks[0]["downloaded"], Value::Null);
    let second = &tracks[1];
    assert_eq!(second["language"], "fra");
    assert_eq!(second["codec"], "sub_rip");
    assert_eq!(second["title"], "Alien.1979.1080p (SDH)");
    assert_eq!(second["path"], Value::Null, "the cache path never shows");
    assert_eq!(second["downloaded"]["id"], subtitle.id.to_string());
    assert_eq!(second["downloaded"]["provider"], "opensubtitles");
    assert_eq!(
        second["downloaded"]["downloadedBy"],
        f.viewer.user_id.to_string()
    );
}

#[tokio::test]
async fn only_whoever_downloaded_it_or_an_administrator_removes_it() {
    let f = fixture().await;
    let first = downloaded(&f, f.viewer, "file-1").await;
    let second = downloaded(&f, f.viewer, "file-2").await;
    let delete = |id: DownloadedSubtitleId| {
        format!(r#"mutation {{ deleteDownloadedSubtitle(id: "{id}") {{ id }} }}"#)
    };

    let refused = run(&f, f.other, delete(first.id)).await;
    assert_eq!(code(&refused).as_deref(), Some("FORBIDDEN"));
    let unseen = run(&f, f.outsider, delete(first.id)).await;
    assert!(unseen.errors.is_empty(), "{:?}", unseen.errors);
    assert_eq!(
        unseen.data.into_json().unwrap()["deleteDownloadedSubtitle"],
        Value::Null,
        "someone who can't see the title learns nothing"
    );

    let own = run(&f, f.viewer, delete(first.id)).await;
    assert!(own.errors.is_empty(), "{:?}", own.errors);
    assert_eq!(
        own.data.into_json().unwrap()["deleteDownloadedSubtitle"]["id"],
        first.id.to_string()
    );
    let again = run(&f, f.viewer, delete(first.id)).await;
    assert_eq!(
        again.data.into_json().unwrap()["deleteDownloadedSubtitle"],
        Value::Null
    );

    let admin = run(&f, f.admin, delete(second.id)).await;
    assert!(admin.errors.is_empty(), "{:?}", admin.errors);
    assert!(
        f.store
            .downloaded_subtitles_for_media(f.media)
            .await
            .expect("list")
            .is_empty()
    );
}

#[tokio::test]
async fn searching_and_downloading_need_a_title_the_caller_can_play() {
    let f = fixture().await;
    let search = format!(
        r#"{{ searchSubtitles(mediaFileId: "{}", languages: ["fr"]) {{
            offers {{ handle }} failures {{ provider }} }} }}"#,
        f.media
    );
    let found = run(&f, f.viewer, search.clone()).await;
    assert!(found.errors.is_empty(), "{:?}", found.errors);
    assert_eq!(
        found.data.into_json().unwrap()["searchSubtitles"],
        serde_json::json!({ "offers": [], "failures": [] }),
        "no subtitle plugin, nothing found"
    );
    let hidden = run(&f, f.outsider, search).await;
    assert_eq!(hidden.errors[0].message, "media file not found");

    let download = run(
        &f,
        f.viewer,
        format!(
            r#"mutation {{ downloadSubtitle(mediaFileId: "{}", provider: "opensubtitles",
                handle: "file-9") {{ index }} }}"#,
            f.media
        ),
    )
    .await;
    assert_eq!(code(&download).as_deref(), Some("NOT_OFFERED"));
}
