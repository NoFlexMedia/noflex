// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    os::unix::fs::{PermissionsExt, symlink},
    sync::Arc,
};

use axum::{body::to_bytes, http::StatusCode};
use noflex_auth::{JwtConfig, Principal, issue_media_token, verify_media_token};
use noflex_core::{
    EditionId, LibraryId, MediaFileId, MetadataStatus, UserId, UserRole, WorkId, WorkKind,
};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_server::{
    stream::{JwtMediaUrlSigner, byte_route_principal},
    trickplay::{TrickplayError, TrickplayWorker},
    trickplay_serve::{parse_sheet_index, rewrite_vtt_token, serve_sheet, serve_vtt},
};
use noflex_storage::{
    RequestUser, Storage,
    domain::{Chapter, Edition, Library, LibraryKind, MediaFile, User, Work},
};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

fn executable(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).expect("write script");
    let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("chmod");
}

async fn seed_user(storage: &dyn Storage) -> UserId {
    let user = User {
        id: UserId::new(),
        name: format!("viewer-{}", UserId::new()),
        password_hash: String::new(),
        is_admin: false,
        role: UserRole::User,
        created_at: OffsetDateTime::now_utc(),
    };
    storage.upsert_user(&user).await.expect("user");
    user.id
}

async fn seed(root: &std::path::Path) -> (Arc<dyn Storage>, Edition, MediaFile) {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let now = OffsetDateTime::now_utc();
    let library = Library {
        id: LibraryId::new(),
        name: "Films".to_string(),
        kind: LibraryKind::Movies,
        paths: vec![root.to_string_lossy().into_owned()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let work = Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Long Movie".to_string(),
        sort_title: "long movie".to_string(),
        year: None,
        release_date: None,
        overview: None,
        tagline: None,
        original_title: None,
        community_rating: None,
        external_ids: Vec::new(),
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: MetadataStatus::Unconfirmed,
        locked_fields: Vec::new(),
        added_at: now,
        updated_at: now,
    };
    store.upsert_work(&work).await.expect("work");
    let edition = Edition {
        id: EditionId::new(),
        work_id: work.id,
        name: "Default".to_string(),
        runtime_ms: Some(1_010_000),
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![Chapter {
            position_ms: 1_000_500,
            title: Some("Finale".to_string()),
        }],
        codec_video: Some("h264".to_string()),
        codec_audio: None,
        width: Some(1920),
        height: Some(1080),
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
    store.upsert_edition(&edition).await.expect("edition");
    let source = root.join("movie.mkv");
    std::fs::write(&source, b"video").expect("source");
    let media = MediaFile {
        id: MediaFileId::new(),
        edition_id: edition.id,
        path: source.to_string_lossy().into_owned(),
        part_index: 0,
        size_bytes: 5,
        mtime: now,
        container: Some("matroska".to_string()),
        probe: serde_json::json!({}).into(),
        subtitle_tracks: Vec::new(),
    };
    store.upsert_media_file(&media).await.expect("media");
    (Arc::new(store), edition, media)
}

#[tokio::test]
async fn worker_uses_deterministic_ffmpeg_argv_and_persists_rollover() {
    let temp = tempfile::tempdir().expect("temp");
    let ffmpeg = temp.path().join("ffmpeg");
    executable(
        &ffmpeg,
        "#!/bin/sh\nlog=\"$(dirname \"$0\")/argv\"\n: > \"$log\"\nout=\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\" >> \"$log\"; out=$arg; done\nprintf jpeg > \"$out\"\n",
    );
    let (storage, edition, _) = seed(temp.path()).await;
    let worker = TrickplayWorker::new(storage.clone(), ffmpeg, temp.path().join("cache"));
    let asset = worker.generate(&edition).await.expect("generate");

    assert_eq!(asset.sheets.len(), 2);
    assert_eq!(asset.sheets[0].tile_count, 100);
    assert_eq!(asset.sheets[1].tile_count, 1);
    assert_eq!(asset.chapter_cues[0].sheet_index, 1);
    assert_eq!(asset.chapter_cues[0].x, 0);
    assert!(std::path::Path::new(&asset.vtt_path).is_file());
    let argv = std::fs::read_to_string(temp.path().join("argv")).expect("argv");
    assert!(argv.contains("1000.000\n"));
    assert!(argv.contains("fps=1/10:start_time=0,scale=160:-2,tile=10x10:nb_frames=1"));
    assert!(
        storage
            .get_trickplay(edition.id)
            .await
            .expect("stored")
            .is_some()
    );
}

#[tokio::test]
async fn failed_stale_regeneration_keeps_old_asset_for_later_retry() {
    let temp = tempfile::tempdir().expect("temp");
    let good = temp.path().join("ffmpeg-good");
    executable(
        &good,
        "#!/bin/sh\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf jpeg > \"$out\"\n",
    );
    let bad = temp.path().join("ffmpeg-bad");
    executable(&bad, "#!/bin/sh\nexit 9\n");
    let (storage, edition, mut media) = seed(temp.path()).await;
    let cache = temp.path().join("cache");
    let first = TrickplayWorker::new(storage.clone(), good.clone(), cache.clone())
        .generate(&edition)
        .await
        .expect("first");
    let first_vtt = std::path::PathBuf::from(&first.vtt_path);

    media.mtime += time::Duration::seconds(1);
    storage
        .upsert_media_file(&media)
        .await
        .expect("stale media");
    TrickplayWorker::new(storage.clone(), bad, cache.clone())
        .run_sweep()
        .await;
    let retained = storage
        .get_trickplay(edition.id)
        .await
        .expect("retained")
        .expect("old asset");
    assert_eq!(retained.generated_at, first.generated_at);
    assert!(first_vtt.is_file());

    let edition_cache = first_vtt
        .parent()
        .and_then(std::path::Path::parent)
        .expect("edition cache");
    let orphan = edition_cache.join("orphan-version");
    std::fs::create_dir(&orphan).expect("orphan");
    std::fs::write(orphan.join("stale"), b"stale").expect("orphan file");
    let outside = temp.path().join("outside-cache");
    std::fs::create_dir(&outside).expect("outside");
    let linked = edition_cache.join("linked-version");
    symlink(&outside, &linked).expect("symlink");

    TrickplayWorker::new(storage.clone(), good, cache)
        .run_sweep()
        .await;
    let refreshed = storage
        .get_trickplay(edition.id)
        .await
        .expect("refreshed")
        .expect("asset");
    assert_eq!(refreshed.source_media_mtime, media.mtime);
    assert!(!first_vtt.exists());
    assert!(!orphan.exists());
    assert!(linked.is_symlink());
    assert!(outside.is_dir());
}

#[tokio::test]
async fn serving_honors_range_mime_and_validates_sheet_index() {
    let temp = tempfile::tempdir().expect("temp");
    let ffmpeg = temp.path().join("ffmpeg");
    executable(
        &ffmpeg,
        "#!/bin/sh\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf jpeg-data > \"$out\"\n",
    );
    let (storage, edition, _) = seed(temp.path()).await;
    TrickplayWorker::new(storage.clone(), ffmpeg, temp.path().join("cache"))
        .generate(&edition)
        .await
        .expect("generate");

    let cache_root = temp.path().join("cache/trickplay");
    let response = serve_vtt(&storage, &cache_root, edition.id, Some("bytes=0-5"), None).await;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-type"], "text/vtt");
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(&body[..], b"WEBVTT");

    let jwt = JwtConfig::for_server(temp.path()).expect("JWT config");
    let viewer = seed_user(storage.as_ref()).await;
    let token = issue_media_token(
        &jwt,
        viewer,
        None,
        &format!("trickplay:{}", edition.id),
        std::time::Duration::from_secs(60),
    )
    .expect("token");
    let response = serve_vtt(
        &storage,
        &cache_root,
        edition.id,
        Some("bytes=0-5"),
        Some(&token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/vtt");
    let body = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("rewritten body")
            .to_vec(),
    )
    .expect("utf8");
    assert!(body.contains(&format!("?token={token}#xywh=")));
    assert!(!body.contains(&format!("#xywh=?token={token}")));

    let (principal, validated) = byte_route_principal(
        storage.as_ref(),
        &Principal::anonymous(),
        &jwt,
        Some(&token),
        &format!("trickplay:{}", edition.id),
    )
    .await
    .expect("sheet token");
    assert_eq!(principal.user_id, viewer);
    assert_eq!(validated.as_deref(), Some(token.as_str()));
    let response = serve_sheet(&storage, &cache_root, edition.id, 0, Some("bytes=0-3")).await;
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    assert_eq!(
        rewrite_vtt_token("image.jpg#xywh=0,0,1,1\n", "a+b/c="),
        "image.jpg?token=a%2Bb%2Fc%3D#xywh=0,0,1,1\n"
    );

    let response = serve_sheet(&storage, &cache_root, edition.id, 999, None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(parse_sheet_index("0"), Some(0));
    assert_eq!(parse_sheet_index("../0"), None);
    assert_eq!(parse_sheet_index("%2e%2e"), None);

    let mut asset = storage
        .get_trickplay(edition.id)
        .await
        .expect("asset")
        .expect("asset exists");
    let outside = temp.path().join("outside.vtt");
    std::fs::write(&outside, "WEBVTT\n").expect("outside vtt");
    asset.vtt_path = outside.to_string_lossy().into_owned();
    storage
        .upsert_trickplay(&asset)
        .await
        .expect("replace asset");
    assert_eq!(
        serve_vtt(&storage, &cache_root, edition.id, None, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn graphql_trickplay_urls_only_carry_tokens_for_authenticated_requests() {
    let temp = tempfile::tempdir().expect("temp");
    let ffmpeg = temp.path().join("ffmpeg");
    executable(
        &ffmpeg,
        "#!/bin/sh\nout=\nfor arg in \"$@\"; do out=$arg; done\nprintf jpeg > \"$out\"\n",
    );
    let (storage, edition, _) = seed(temp.path()).await;
    TrickplayWorker::new(storage.clone(), ffmpeg, temp.path().join("cache"))
        .generate(&edition)
        .await
        .expect("generate");
    let jwt = Arc::new(JwtConfig::for_server(temp.path()).expect("JWT config"));
    let schema = build_schema(SchemaContext {
        storage: Some(storage),
        media_url_signer: Some(Arc::new(JwtMediaUrlSigner::new(jwt.clone()))),
        ..Default::default()
    });
    let query = format!(
        "{{ editions(workId: \"{}\") {{ trickplay {{ vttUrl sheets {{ url }} chapterCues {{ sheetUrl }} }} }} }}",
        edition.work_id
    );

    let anonymous = schema.execute(query.clone()).await;
    assert!(!anonymous.errors.is_empty(), "anonymous request must fail");

    let caller = UserId::new();
    let authenticated = schema
        .execute(async_graphql::Request::new(query).data(RequestUser {
            user_id: caller,
            is_admin: true,
        }))
        .await;
    assert!(
        authenticated.errors.is_empty(),
        "authenticated: {:?}",
        authenticated.errors
    );
    let authenticated = authenticated.data.into_json().expect("authenticated data");
    let url = authenticated["editions"][0]["trickplay"]["vttUrl"]
        .as_str()
        .expect("vtt URL");
    let token = url.split_once("?token=").expect("token query").1;
    assert_eq!(
        verify_media_token(&jwt, token).map(|claims| (claims.target, claims.user)),
        Some((format!("trickplay:{}", edition.id), caller))
    );
}

#[tokio::test]
async fn trickplay_media_token_is_scoped_to_one_edition_and_a_live_user() {
    let temp = tempfile::tempdir().expect("temp");
    let (storage, _, _) = seed(temp.path()).await;
    let jwt = JwtConfig::for_server(temp.path()).expect("JWT config");
    let viewer = seed_user(storage.as_ref()).await;
    let edition = EditionId::new();
    let mint = |user| {
        issue_media_token(
            &jwt,
            user,
            None,
            &format!("trickplay:{edition}"),
            std::time::Duration::from_secs(60),
        )
        .expect("token")
    };
    let token = mint(viewer);
    let resolve = |token: String, target: String| {
        let storage = storage.clone();
        let jwt = jwt.clone();
        async move {
            byte_route_principal(
                storage.as_ref(),
                &Principal::anonymous(),
                &jwt,
                Some(&token),
                &target,
            )
            .await
            .map(|(principal, _)| principal.user_id)
        }
    };
    assert_eq!(
        resolve(token.clone(), format!("trickplay:{edition}")).await,
        Ok(viewer)
    );
    assert_eq!(
        resolve(token, format!("trickplay:{}", EditionId::new())).await,
        Err(StatusCode::UNAUTHORIZED)
    );
    assert_eq!(
        resolve(mint(UserId::new()), format!("trickplay:{edition}")).await,
        Err(StatusCode::UNAUTHORIZED),
        "token for a deleted/unknown user"
    );
}

#[tokio::test]
async fn non_video_editions_are_not_rendered() {
    let temp = tempfile::tempdir().expect("temp");
    let (storage, edition, _) = seed(temp.path()).await;
    let mut work = storage
        .get_work(edition.work_id)
        .await
        .expect("work")
        .expect("work exists");
    work.kind = WorkKind::Track;
    storage.upsert_work(&work).await.expect("retag work");
    let result = TrickplayWorker::new(
        storage,
        temp.path().join("missing-ffmpeg"),
        temp.path().join("cache"),
    )
    .generate(&edition)
    .await;
    assert!(
        matches!(result, Err(TrickplayError::NotVideo)),
        "{result:?}"
    );
}
