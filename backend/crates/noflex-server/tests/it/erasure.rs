// SPDX-License-Identifier: AGPL-3.0-only

use std::{path::Path, sync::Arc};

use async_graphql::Request;
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{SessionId, UserId};
use noflex_graphql::{NoflexSchema, SchemaContext, build_schema};
use noflex_server::{
    config::Config,
    erasure::{PurgeWorker, ResetPaths, finish_reset},
};
use noflex_storage::{
    RequestUser,
    domain::{
        ActivityKind, Edition, ExportFormat, ExportJob, ExportJobStatus, Library, LibraryKind,
        SmartTranscodingSettings, Work,
    },
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

async fn open(path: &Path) -> Arc<TursoStore> {
    let store = Arc::new(TursoStore::open(path).await.expect("open"));
    store.migrate().await.expect("migrate");
    store
}

fn schema(store: &Arc<TursoStore>, jwt: &Arc<JwtConfig>) -> NoflexSchema {
    build_schema(SchemaContext {
        storage: Some(store.clone()),
        jwt: Some(jwt.clone()),
        ..SchemaContext::default()
    })
}

async fn run(
    schema: &NoflexSchema,
    principal: Option<Principal>,
    query: &str,
) -> serde_json::Value {
    let mut request = Request::new(query);
    if let Some(principal) = principal {
        request = request.data(principal).data(RequestUser {
            user_id: principal.user_id,
            is_admin: principal.is_admin,
        });
    }
    let response = schema.execute(request).await;
    assert!(response.errors.is_empty(), "{query}: {:?}", response.errors);
    response.data.into_json().expect("json")
}

/// Signed in as `username`, the way a request through the server would be.
async fn sign_in(schema: &NoflexSchema, jwt: &JwtConfig, username: &str) -> Principal {
    let data = run(
        schema,
        None,
        &format!(r#"mutation {{ login(username: "{username}", password: "pw") {{ token }} }}"#),
    )
    .await;
    let claims = verify_jwt(jwt, data["login"]["token"].as_str().expect("token")).expect("claims");
    Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("sub")),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(claims.sid.parse().expect("sid"))),
    }
}

async fn admin_and_viewer(schema: &NoflexSchema, jwt: &JwtConfig) -> (Principal, Principal) {
    run(
        schema,
        None,
        r#"mutation { completeSetupWizard(username: "admin", password: "pw") { userId } }"#,
    )
    .await;
    let admin = sign_in(schema, jwt, "admin").await;
    run(
        schema,
        Some(admin),
        r#"mutation { createUser(username: "viewer", password: "pw", role: USER) { id } }"#,
    )
    .await;
    let viewer = sign_in(schema, jwt, "viewer").await;
    (admin, viewer)
}

/// A film in a library rooted at `media`, for export jobs to name.
async fn seed_edition(store: &TursoStore, media: &Path) -> Edition {
    let now = OffsetDateTime::now_utc();
    let library = Library {
        id: noflex_core::LibraryId::new(),
        name: "Films".to_string(),
        kind: LibraryKind::Movies,
        paths: vec![media.to_string_lossy().into_owned()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let work = Work {
        id: noflex_core::WorkId::new(),
        kind: noflex_core::WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Alien".to_string(),
        sort_title: "alien".to_string(),
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
        id: noflex_core::EditionId::new(),
        work_id: work.id,
        name: "Default".to_string(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    store.upsert_edition(&edition).await.expect("edition");
    edition
}

fn finished_export(user: UserId, edition: &Edition, output: &Path) -> ExportJob {
    let now = OffsetDateTime::now_utc();
    ExportJob {
        id: noflex_core::ExportJobId::new(),
        user_id: user,
        edition_id: edition.id,
        format: ExportFormat::Mp4,
        status: ExportJobStatus::Completed,
        progress: 1.0,
        output_path: Some(output.to_string_lossy().into_owned()),
        error: None,
        created_at: now,
        started_at: Some(now),
        finished_at: Some(now),
    }
}

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, b"bytes").expect("write");
}

#[tokio::test]
async fn a_purge_cut_short_by_a_restart_finishes_at_the_next_start() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("noflex.db");
    let own = dir.path().join("downloads/own.mp4");
    let shared = dir.path().join("downloads/shared.mp4");
    touch(&own);
    touch(&shared);
    let jwt = Arc::new(JwtConfig::dev(b"test-secret-32-bytes-please-use-it").expect("jwt"));

    let viewer = {
        let store = open(&db).await;
        let schema = schema(&store, &jwt);
        let (admin, viewer) = admin_and_viewer(&schema, &jwt).await;
        let edition = seed_edition(&store, &dir.path().join("media")).await;
        for (user, output) in [
            (viewer.user_id, &own),
            (viewer.user_id, &shared),
            (admin.user_id, &shared),
        ] {
            store
                .create_export_job(&finished_export(user, &edition, output))
                .await
                .expect("export");
        }
        let now = OffsetDateTime::now_utc();
        for actor in [viewer.user_id, admin.user_id] {
            store
                .append_activity(
                    ActivityKind::PlayStarted,
                    Some(&actor.to_string()),
                    Some(&edition.work_id.to_string()),
                    None,
                    now,
                )
                .await
                .expect("activity");
        }
        let data = run(
            &schema,
            Some(viewer),
            r#"mutation { deleteAccount(currentPassword: "pw") { serverErased } }"#,
        )
        .await;
        assert_eq!(data["deleteAccount"]["serverErased"], false);
        // The server stops here, before its purge worker gets to it.
        viewer
    };

    let store = open(&db).await;
    assert!(store.get_user(viewer.user_id).await.expect("get").is_none());
    let worker = PurgeWorker {
        storage: store.clone(),
        erasure: Arc::default(),
    };
    assert_eq!(worker.run_pending().await.expect("purge"), 1);
    assert!(!own.exists(), "their own copy goes");
    assert!(shared.exists(), "a copy someone else's job shares stays");
    let actors = store
        .list_activity(&[], None, 100, None)
        .await
        .expect("activity")
        .items
        .into_iter()
        .filter_map(|row| row.actor)
        .collect::<Vec<_>>();
    assert!(!actors.contains(&viewer.user_id.to_string()));
    assert!(!actors.is_empty(), "the administrator's history stays");
    assert_eq!(worker.run_pending().await.expect("nothing left"), 0);
}

#[tokio::test]
async fn erasing_the_server_lands_at_first_run_setup_and_keeps_the_media() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = Config {
        data_dir: dir.path().join("data"),
        cache_dir: dir.path().join("cache"),
        ..Config::default()
    };
    let db = cfg.data_dir.join("noflex.db");
    std::fs::create_dir_all(&cfg.data_dir).expect("data dir");
    let media = dir.path().join("media/Alien (1979)/Alien (1979).mkv");
    let version = format!("{}.mp4", uuid::Uuid::now_v7());
    let chosen = dir.path().join("chosen");
    let made = [
        cfg.image_dir().join("posters/alien.jpg"),
        cfg.transcode_dir().join("session/segment.ts"),
        cfg.embed_dir().join("vectors.bin"),
        cfg.download_dir().join("alien.mp4"),
        cfg.recordings_dir().join("news.mkv"),
        cfg.optimized_dir().join(&version),
        chosen.join(&version),
        cfg.data_dir.join("jwt.key"),
    ];
    let kept = [
        media.clone(),
        cfg.optimized_dir().join("notes.txt"),
        chosen.join("holiday.mp4"),
    ];
    for path in made.iter().chain(&kept) {
        touch(path);
    }
    let jwt = Arc::new(JwtConfig::dev(b"test-secret-32-bytes-please-use-it").expect("jwt"));

    {
        let store = open(&db).await;
        let schema = schema(&store, &jwt);
        let (admin, _) = admin_and_viewer(&schema, &jwt).await;
        seed_edition(&store, media.parent().expect("folder")).await;
        store
            .set_smart_transcoding_settings(&SmartTranscodingSettings {
                dir: Some(chosen.to_string_lossy().into_owned()),
                ..SmartTranscodingSettings::default()
            })
            .await
            .expect("settings");
        let data = run(
            &schema,
            Some(admin),
            r#"mutation { deleteAccount(currentPassword: "pw", eraseServer: true) { serverErased } }"#,
        )
        .await;
        assert_eq!(data["deleteAccount"]["serverErased"], true);
        // The restart that follows is cut short once the rows are gone, before
        // anything on disk is.
        store.erase_all_data().await.expect("partial erasure");
    }

    let store = open(&db).await;
    let paths = ResetPaths::for_config(&cfg);
    assert!(finish_reset(store.as_ref(), &paths).await.expect("erase"));
    assert!(!finish_reset(store.as_ref(), &paths).await.expect("done"));
    for path in &made {
        assert!(!path.exists(), "{} should be gone", path.display());
    }
    for path in &kept {
        assert!(path.exists(), "{} should be kept", path.display());
    }
    assert!(
        cfg.image_dir().exists(),
        "a cache folder is emptied, not removed"
    );

    let schema = schema(&store, &jwt);
    let data = run(&schema, None, "{ serverInfo { isConfigured resetting } }").await;
    assert_eq!(
        data["serverInfo"],
        serde_json::json!({ "isConfigured": false, "resetting": false })
    );
    let admin = {
        run(
            &schema,
            None,
            r#"mutation { completeSetupWizard(username: "fresh", password: "pw") { userId } }"#,
        )
        .await;
        sign_in(&schema, &jwt, "fresh").await
    };
    let data = run(
        &schema,
        Some(admin),
        "{ libraries { id } users { username } }",
    )
    .await;
    assert_eq!(
        data,
        serde_json::json!({ "libraries": [], "users": [{ "username": "fresh" }] })
    );
}
