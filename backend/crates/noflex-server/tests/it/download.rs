// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use noflex_server::download::DownloadWorker;
use noflex_storage::store::Storage;
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

/// Build a Library + Work + Edition with one MediaFile on the
/// given store. The MediaFile path is a real file in the
/// supplied `work_dir` so the ffmpeg subprocess can read it.
async fn seed_with_real_file(
    store: &TursoStore,
    work_dir: &std::path::Path,
) -> (
    noflex_core::UserId,
    noflex_core::EditionId,
    std::path::PathBuf,
) {
    let now = OffsetDateTime::now_utc();

    let user = noflex_storage::domain::User {
        id: noflex_core::UserId::new(),
        name: "downloader".to_string(),
        password_hash: "argon2id$placeholder".to_string(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: now,
    };
    store.upsert_user(&user).await.expect("upsert user");

    let library = noflex_storage::domain::Library {
        id: noflex_core::LibraryId::new(),
        name: "Films".to_string(),
        kind: noflex_storage::domain::LibraryKind::Movies,
        paths: vec![work_dir.to_string_lossy().to_string()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");

    let work = noflex_storage::domain::Work {
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
        added_at: now,
        updated_at: now,
    };
    store.upsert_work(&work).await.expect("work");

    let edition = noflex_storage::domain::Edition {
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

    let input_path = work_dir.join("alien.mkv");
    let payload = b"this would be an mkv, but the test uses a copy script";
    std::fs::write(&input_path, payload).expect("write input file");

    let mf = noflex_storage::domain::MediaFile {
        id: noflex_core::MediaFileId::new(),
        edition_id: edition.id,
        path: input_path.to_string_lossy().to_string(),
        part_index: 0,
        size_bytes: payload.len() as u64,
        mtime: now,
        container: Some("matroska".to_string()),
        probe: serde_json::json!({}).into(),
        subtitle_tracks: Vec::new(),
    };
    store.upsert_media_file(&mf).await.expect("media");

    (user.id, edition.id, input_path)
}

/// Build a `ffmpeg` mock binary that copies its first `-i`
/// argument to its last positional argument. Mirrors the
/// pattern in `noflex-transcoder/src/spawn.rs::make_mock_ffmpeg`.
fn make_mock_ffmpeg() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("noflex-mock-ffmpeg-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).expect("mock dir");
    let mock = dir.join("ffmpeg");
    let script = "\
#!/bin/sh
# Tiny ffmpeg stand-in: copy -i $INPUT to the last argv (the
# output path). The download worker's argv puts the output
# last, so this is enough to fake a successful remux.
INPUT=\"\"
OUT=\"\"
for arg in \"$@\"; do
  case \"$arg\" in
    -i) shift_arg=1 ;;
    -*) shift_arg=0 ;;
    *)
      if [ \"$shift_arg\" = \"1\" ]; then
        INPUT=\"$arg\"
        shift_arg=0
      else
        OUT=\"$arg\"
      fi
      ;;
  esac
done
if [ -z \"$INPUT\" ] || [ -z \"$OUT\" ]; then
  echo \"mock ffmpeg: missing input or output\" >&2
  exit 1
fi
cp \"$INPUT\" \"$OUT\"
exit 0
";
    std::fs::write(&mock, script).expect("write mock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&mock).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&mock, perms).unwrap();
    }
    mock
}

#[tokio::test]
async fn worker_processes_queued_job_to_completed() {
    let work_dir = std::env::temp_dir().join(format!("noflex-dl-work-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&work_dir).expect("work dir");
    let output_dir = std::env::temp_dir().join(format!("noflex-dl-out-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&output_dir).expect("output dir");

    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let (user_id, edition_id, input_path) = seed_with_real_file(&store, &work_dir).await;

    let job = noflex_storage::domain::ExportJob {
        id: noflex_core::ExportJobId::new(),
        user_id,
        edition_id,
        format: noflex_storage::domain::ExportFormat::Mp4,
        status: noflex_storage::domain::ExportJobStatus::Queued,
        progress: 0.0,
        output_path: None,
        error: None,
        created_at: OffsetDateTime::now_utc(),
        started_at: None,
        finished_at: None,
    };
    let job_id = job.id;
    let store_arc: Arc<dyn Storage> = Arc::new(store);
    store_arc.create_export_job(&job).await.expect("create job");

    let mock_ffmpeg = make_mock_ffmpeg();
    let mut worker = DownloadWorker::new(store_arc.clone(), mock_ffmpeg, output_dir.clone());
    let processed = worker.tick().await.expect("worker tick");
    assert!(processed, "expected the queued job to be processed");

    let finished = store_arc
        .get_export_job(job_id)
        .await
        .expect("get job")
        .expect("job present");
    assert_eq!(
        finished.status,
        noflex_storage::domain::ExportJobStatus::Completed
    );
    assert!((finished.progress - 1.0).abs() < f32::EPSILON);
    assert!(
        finished.output_path.is_some(),
        "output_path must be stamped"
    );
    let output_path = std::path::PathBuf::from(finished.output_path.unwrap());
    assert!(output_path.exists(), "output file must exist on disk");
    let output_bytes = std::fs::read(&output_path).expect("read output");
    let input_bytes = std::fs::read(&input_path).expect("read input");
    assert_eq!(output_bytes, input_bytes, "mock copies input verbatim");
    assert!(finished.finished_at.is_some());
}

#[tokio::test]
async fn running_cancellation_kills_worker_process() {
    let work_dir = std::env::temp_dir().join(format!("noflex-dl-cancel-{}", uuid::Uuid::now_v7()));
    let output_dir = work_dir.join("output");
    std::fs::create_dir_all(&output_dir).expect("output dir");
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let (user_id, edition_id, _) = seed_with_real_file(&store, &work_dir).await;
    let job = noflex_storage::domain::ExportJob {
        id: noflex_core::ExportJobId::new(),
        user_id,
        edition_id,
        format: noflex_storage::domain::ExportFormat::Mkv,
        status: noflex_storage::domain::ExportJobStatus::Queued,
        progress: 0.0,
        output_path: None,
        error: None,
        created_at: OffsetDateTime::now_utc(),
        started_at: None,
        finished_at: None,
    };
    let job_id = job.id;
    let storage: Arc<dyn Storage> = Arc::new(store);
    storage.create_export_job(&job).await.expect("create job");

    let mock_dir = work_dir.join("mock");
    std::fs::create_dir_all(&mock_dir).expect("mock dir");
    let mock = mock_dir.join("ffmpeg");
    std::fs::write(&mock, "#!/bin/sh\nwhile true; do sleep 1; done\n").expect("mock");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&mock).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&mock, permissions).unwrap();
    }

    let mut worker = DownloadWorker::new(storage.clone(), mock, output_dir);
    let worker_task = tokio::spawn(async move { worker.tick().await });
    for _ in 0..50 {
        if storage
            .get_export_job(job_id)
            .await
            .unwrap()
            .unwrap()
            .status
            == noflex_storage::domain::ExportJobStatus::Running
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let cancellation = storage
        .request_export_job_cancellation(job_id, user_id)
        .await
        .unwrap()
        .unwrap();
    assert!(cancellation.changed);
    assert_eq!(
        cancellation.job.status,
        noflex_storage::domain::ExportJobStatus::CancelRequested
    );
    tokio::time::timeout(std::time::Duration::from_secs(6), worker_task)
        .await
        .expect("worker observed cancellation")
        .expect("worker task")
        .expect("worker tick");
    assert_eq!(
        storage
            .get_export_job(job_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        noflex_storage::domain::ExportJobStatus::Cancelled
    );
}

#[tokio::test]
async fn worker_marks_missing_media_file_as_failed() {
    let work_dir = std::env::temp_dir().join(format!("noflex-dl-nomedia-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&work_dir).expect("work dir");
    let output_dir =
        std::env::temp_dir().join(format!("noflex-dl-nomedia-out-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&output_dir).expect("output dir");

    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let now = OffsetDateTime::now_utc();
    let user = noflex_storage::domain::User {
        id: noflex_core::UserId::new(),
        name: "x".to_string(),
        password_hash: "argon2id$placeholder".to_string(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: now,
    };
    store.upsert_user(&user).await.expect("user");

    let library = noflex_storage::domain::Library {
        id: noflex_core::LibraryId::new(),
        name: "L".to_string(),
        kind: noflex_storage::domain::LibraryKind::Movies,
        paths: vec![work_dir.to_string_lossy().to_string()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let work = noflex_storage::domain::Work {
        id: noflex_core::WorkId::new(),
        kind: noflex_core::WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "X".to_string(),
        sort_title: "x".to_string(),
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
        added_at: now,
        updated_at: now,
    };
    store.upsert_work(&work).await.expect("work");
    let edition = noflex_storage::domain::Edition {
        id: noflex_core::EditionId::new(),
        work_id: work.id,
        name: "X".to_string(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    store.upsert_edition(&edition).await.expect("edition");

    let job = noflex_storage::domain::ExportJob {
        id: noflex_core::ExportJobId::new(),
        user_id: user.id,
        edition_id: edition.id,
        format: noflex_storage::domain::ExportFormat::Mp4,
        status: noflex_storage::domain::ExportJobStatus::Queued,
        progress: 0.0,
        output_path: None,
        error: None,
        created_at: now,
        started_at: None,
        finished_at: None,
    };
    let job_id = job.id;
    let store_arc: Arc<dyn Storage> = Arc::new(store);
    store_arc.create_export_job(&job).await.expect("create job");

    let mock_ffmpeg = make_mock_ffmpeg();
    let mut worker = DownloadWorker::new(store_arc.clone(), mock_ffmpeg, output_dir);
    let _ = worker.tick().await.expect("worker tick");

    let after = store_arc
        .get_export_job(job_id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(
        after.status,
        noflex_storage::domain::ExportJobStatus::Failed
    );
    let err = after.error.unwrap_or_default();
    assert!(err.contains("no media file"), "got: {err}");
}

// ---- T26: byte-range download handler --------------------------------

fn principal_for(user_id: noflex_core::UserId, is_admin: bool) -> noflex_auth::Principal {
    noflex_auth::Principal {
        user_id,
        is_admin,
        session_id: None,
    }
}

/// Insert a pre-completed export job whose `output_path`
/// points to a real file on disk. The caller must have
/// already inserted a user + edition (FK constraints).
async fn insert_completed_job(
    store: &Arc<dyn Storage>,
    user: noflex_core::UserId,
    edition: noflex_core::EditionId,
    output_path: &std::path::Path,
) -> noflex_core::ExportJobId {
    let job = noflex_storage::domain::ExportJob {
        id: noflex_core::ExportJobId::new(),
        user_id: user,
        edition_id: edition,
        format: noflex_storage::domain::ExportFormat::Mp4,
        status: noflex_storage::domain::ExportJobStatus::Completed,
        progress: 1.0,
        output_path: Some(output_path.to_string_lossy().to_string()),
        error: None,
        created_at: OffsetDateTime::now_utc(),
        started_at: Some(OffsetDateTime::now_utc()),
        finished_at: Some(OffsetDateTime::now_utc()),
    };
    let id = job.id;
    store.create_export_job(&job).await.expect("create");
    id
}

#[tokio::test]
async fn serve_full_file_returns_200() {
    use noflex_server::download_serve::serve_download_job;
    let work_dir = std::env::temp_dir().join(format!("noflex-srv-full-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&work_dir).expect("work dir");
    let out_path = work_dir.join("movie.mp4");
    let payload: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    std::fs::write(&out_path, &payload).expect("write");

    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let (user_id, edition_id, _input_path) = seed_with_real_file(&store, &work_dir).await;
    let store_arc: Arc<dyn Storage> = Arc::new(store);
    let job_id = insert_completed_job(&store_arc, user_id, edition_id, &out_path).await;

    let root = noflex_server::file_serve::RootCapability::new(&work_dir)
        .await
        .expect("download root");
    let resp = serve_download_job(
        &store_arc,
        &principal_for(user_id, false),
        job_id,
        &root,
        None,
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(body.len(), payload.len());
    assert_eq!(&body[..], &payload[..]);
}

#[tokio::test]
async fn completed_job_outside_download_root_is_rejected() {
    use noflex_server::{download_serve::serve_download_job, file_serve::RootCapability};

    let temp = tempfile::tempdir().expect("temp");
    let root_dir = temp.path().join("downloads");
    std::fs::create_dir(&root_dir).expect("download root");
    let outside = temp.path().join("outside.mp4");
    std::fs::write(&outside, b"outside").expect("outside");
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let (user_id, edition_id, _) = seed_with_real_file(&store, &root_dir).await;
    let store: Arc<dyn Storage> = Arc::new(store);
    let job_id = insert_completed_job(&store, user_id, edition_id, &outside).await;
    let root = RootCapability::new(&root_dir).await.expect("root");

    let response =
        serve_download_job(&store, &principal_for(user_id, false), job_id, &root, None).await;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_finished_export_stays_put_on_a_server_that_refuses_downloads() {
    use noflex_server::{download_serve::serve_download_job, file_serve::RootCapability};

    let temp = tempfile::tempdir().expect("temp");
    let output = temp.path().join("movie.mp4");
    std::fs::write(&output, b"movie").expect("output");
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let (user_id, edition_id, _) = seed_with_real_file(&store, temp.path()).await;
    let store: Arc<dyn Storage> = Arc::new(store);
    let job_id = insert_completed_job(&store, user_id, edition_id, &output).await;
    let root = RootCapability::new(temp.path()).await.expect("root");
    store.set_downloads_allowed(false).await.expect("turn off");

    let response =
        serve_download_job(&store, &principal_for(user_id, true), job_id, &root, None).await;
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn serve_byte_range_returns_206_with_first_100_bytes() {
    use noflex_server::download_serve::serve_download_job;
    let work_dir = std::env::temp_dir().join(format!("noflex-srv-range-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&work_dir).expect("work dir");
    let out_path = work_dir.join("movie.mp4");
    let payload: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    std::fs::write(&out_path, &payload).expect("write");

    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let (user_id, edition_id, _input_path) = seed_with_real_file(&store, &work_dir).await;
    let store_arc: Arc<dyn Storage> = Arc::new(store);
    let job_id = insert_completed_job(&store_arc, user_id, edition_id, &out_path).await;

    let resp = serve_download_job(
        &store_arc,
        &principal_for(user_id, false),
        job_id,
        &noflex_server::file_serve::RootCapability::new(&work_dir)
            .await
            .expect("download root"),
        Some("bytes=0-99"),
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::PARTIAL_CONTENT);
    let cr = resp
        .headers()
        .get(axum::http::header::CONTENT_RANGE)
        .expect("content-range present")
        .to_str()
        .unwrap()
        .to_string();
    assert!(cr.starts_with("bytes 0-99/"), "got: {cr}");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(body.len(), 100);
    assert_eq!(&body[..], &payload[..100]);
}

#[tokio::test]
async fn serve_unknown_job_returns_404() {
    use noflex_server::download_serve::serve_download_job;
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let store_arc: Arc<dyn Storage> = Arc::new(store);
    let root = noflex_server::file_serve::RootCapability::new(std::env::temp_dir())
        .await
        .expect("download root");
    let resp = serve_download_job(
        &store_arc,
        &principal_for(noflex_core::UserId::new(), false),
        noflex_core::ExportJobId::new(),
        &root,
        None,
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn serve_non_completed_job_returns_409() {
    use noflex_server::download_serve::serve_download_job;
    let work_dir =
        std::env::temp_dir().join(format!("noflex-srv-running-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&work_dir).expect("work dir");
    let out_path = work_dir.join("movie.mp4");
    std::fs::write(&out_path, b"placeholder").expect("write");

    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let (user_id, edition_id, _input_path) = seed_with_real_file(&store, &work_dir).await;
    let store_arc: Arc<dyn Storage> = Arc::new(store);
    let now = OffsetDateTime::now_utc();
    let job = noflex_storage::domain::ExportJob {
        id: noflex_core::ExportJobId::new(),
        user_id,
        edition_id,
        format: noflex_storage::domain::ExportFormat::Mp4,
        status: noflex_storage::domain::ExportJobStatus::Running,
        progress: 0.5,
        output_path: Some(out_path.to_string_lossy().to_string()),
        error: None,
        created_at: now,
        started_at: Some(now),
        finished_at: None,
    };
    let id = job.id;
    store_arc.create_export_job(&job).await.expect("create");

    let root = noflex_server::file_serve::RootCapability::new(&work_dir)
        .await
        .expect("download root");
    let resp =
        serve_download_job(&store_arc, &principal_for(user_id, false), id, &root, None).await;
    assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
}

/// Seed a store with one completed job over a real 1024-byte file;
/// returns the store, the owner's user id, and the job id.
async fn seed_completed_job(
    tag: &str,
) -> (
    Arc<dyn Storage>,
    noflex_core::UserId,
    noflex_core::ExportJobId,
    noflex_server::file_serve::RootCapability,
) {
    let work_dir = std::env::temp_dir().join(format!("noflex-srv-{tag}-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&work_dir).expect("work dir");
    let out_path = work_dir.join("movie.mp4");
    let payload: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
    std::fs::write(&out_path, &payload).expect("write");

    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let (user_id, edition_id, _input_path) = seed_with_real_file(&store, &work_dir).await;
    let store_arc: Arc<dyn Storage> = Arc::new(store);
    let job_id = insert_completed_job(&store_arc, user_id, edition_id, &out_path).await;
    let root = noflex_server::file_serve::RootCapability::new(&work_dir)
        .await
        .expect("download root");
    (store_arc, user_id, job_id, root)
}

#[tokio::test]
async fn serve_unsatisfiable_range_returns_416() {
    use noflex_server::download_serve::serve_download_job;
    let (store, user_id, job_id, root) = seed_completed_job("range416").await;

    let resp = serve_download_job(
        &store,
        &principal_for(user_id, false),
        job_id,
        &root,
        Some("bytes=2000-"),
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::RANGE_NOT_SATISFIABLE);
    let cr = resp
        .headers()
        .get(axum::http::header::CONTENT_RANGE)
        .expect("content-range present")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(cr, "bytes */1024");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    assert!(body.is_empty());
}

#[tokio::test]
async fn serve_malformed_range_falls_back_to_full_body() {
    use noflex_server::download_serve::serve_download_job;
    let (store, user_id, job_id, root) = seed_completed_job("rangemal").await;

    let resp = serve_download_job(
        &store,
        &principal_for(user_id, false),
        job_id,
        &root,
        Some("bytes=0-10,20-30"),
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(body.len(), 1024);
}

#[tokio::test]
async fn serve_anonymous_principal_returns_401() {
    use noflex_server::download_serve::serve_download_job;
    let (store, _user_id, job_id, root) = seed_completed_job("anon").await;

    let resp = serve_download_job(
        &store,
        &noflex_auth::Principal::default(),
        job_id,
        &root,
        None,
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn serve_other_users_job_returns_404() {
    use noflex_server::download_serve::serve_download_job;
    let (store, _owner, job_id, root) = seed_completed_job("notyours").await;

    let resp = serve_download_job(
        &store,
        &principal_for(noflex_core::UserId::new(), false),
        job_id,
        &root,
        None,
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn serve_admin_reads_other_users_job() {
    use noflex_server::download_serve::serve_download_job;
    let (store, _owner, job_id, root) = seed_completed_job("admin").await;

    let resp = serve_download_job(
        &store,
        &principal_for(noflex_core::UserId::new(), true),
        job_id,
        &root,
        None,
    )
    .await;
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(body.len(), 1024);
}
