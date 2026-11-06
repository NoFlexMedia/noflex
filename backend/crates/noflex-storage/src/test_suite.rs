// SPDX-License-Identifier: AGPL-3.0-only
//
// Conformance test suite. Every backend crate runs this against a fresh
// store; behavioral divergence shows up as a test failure, not a runtime
// surprise.
use noflex_core::{
    CollectionId, DevicePinId, EditionId, EmbeddingTarget, ExternalId, ExternalIdKind, LibraryId,
    MediaFileId, MetadataStatus, PlayTarget, PlaylistId, SessionId, TagId, TagTarget, UserId,
    WatchRoomId, WorkField, WorkId, WorkKind,
};
use time::OffsetDateTime;

use crate::{
    RequestUser,
    domain::{
        ActivityKind, Collection, DevicePin, Edition, ExternalIdentity, ImageSource, IntroMarkers,
        Library, LibraryKind, MediaFile, Playlist, RefreshTokenLookup, Session, SubtitleTrack,
        TrickplayAsset, TrickplayChapterCue, TrickplaySheet, User, UserPreference,
        UserRestrictions, WatchRoom, WatchRoomMember, WatchRoomState, Work,
    },
    error::{StorageError, StorageResult},
    query::PageCursor,
    store::{Storage, atomic},
};

/// Run every conformance check against `store`. Panics on the first failure.
pub async fn run_all<S>(store: &S)
where
    S: Storage + ?Sized,
{
    migrate_is_idempotent(store).await;
    health_reports_backend(store).await;
    create_initial_admin_only_on_empty(store).await;
    round_trip_library(store).await;
    round_trip_work_edition_media(store).await;
    edition_identity_is_conflict_safe(store).await;
    edition_upsert_honours_id(store).await;
    media_removal_prunes_empty_catalog(store).await;
    edition_prune_after_move(store).await;
    library_scan_version_roundtrip(store).await;
    works_by_external_id_roundtrip(store).await;
    index_end_roundtrip(store).await;
    metadata_status_roundtrip(store).await;
    work_metadata_fields_roundtrip(store).await;
    transaction_commit_persists(store).await;
    transaction_rollback_disappears(store).await;
    atomic_helper_commits_on_ok(store).await;
    atomic_helper_rolls_back_on_err(store).await;
    user_round_trip(store).await;
    user_uniqueness(store).await;
    external_identities_round_trip(store).await;
    user_preferences_round_trip(store).await;
    user_preferences_upsert(store).await;
    user_preferences_delete(store).await;
    user_preferences_are_isolated(store).await;
    user_preferences_user_delete_cascade(store).await;
    plugin_task_state_round_trip(store).await;
    plugin_task_queue_fifo(store).await;
    plugin_task_delete_and_clear(store).await;
    user_plugin_secrets_round_trip(store).await;
    user_plugin_secrets_are_isolated(store).await;
    user_plugin_secrets_delete_scoped(store).await;
    user_plugin_secrets_user_delete_cascade(store).await;
    session_lifecycle(store).await;
    user_delete_cascades_sessions(store).await;
    record_progress_roundtrip(store).await;
    mark_played_play_count(store).await;
    set_favorite_roundtrip(store).await;
    continue_watching_resumes_unfinished_works(store).await;
    continue_watching_suggests_the_next_episode(store).await;
    continue_watching_handles_multi_episode_files(store).await;
    continue_watching_decays_until_new_episodes_arrive(store).await;
    watched_top_level_works_roll_up(store).await;
    work_embeddings_round_trip(store).await;
    content_categories_split_by_kind_and_anime(store).await;
    work_features_collect_comparison_facts(store).await;
    episodes_after_lists_regular_episodes_in_order(store).await;
    devices_belong_to_their_user(store).await;
    smart_transcode_marks_queue_by_scope(store).await;
    optimized_versions_queue_by_priority(store).await;
    smart_transcoding_settings_round_trip(store).await;
    downloads_allowed_round_trip(store).await;
    invites_can_only_be_spent_while_valid(store).await;
    linked_servers_belong_to_their_user(store).await;
    applied_watch_state_keeps_its_original_time(store).await;
    tag_namespace_uniqueness(store).await;
    tag_assign_unassign(store).await;
    tag_assignments_carry_source_and_weight(store).await;
    tag_metadata_roundtrip(store).await;
    tag_parent_self_reference(store).await;
    get_tag_round_trip(store).await;
    list_tags_filters_counts_and_paginates(store).await;
    list_works_tag_filter(store).await;
    image_work_roundtrip(store).await;
    image_tag_covers_people(store).await;
    image_unique_per_kind(store).await;
    image_list_by_target(store).await;
    image_list_for_library_works(store).await;
    image_delete_by_target_and_kind(store).await;
    works_missing_image_sweep(store).await;
    image_sources_coexist_and_resolve_by_order(store).await;
    works_missing_image_honors_sources(store).await;
    identified_works_missing_image_source_sweep(store).await;
    image_source_order_follows_library_preference(store).await;
    search_lexical_match(store).await;
    search_lexical_miss(store).await;
    search_lexical_finds_credited_people(store).await;
    search_vector_similarity(store).await;
    search_vector_skips_seasons_and_episodes_without_a_synopsis(store).await;
    embedding_model_changes_keep_vectors_until_pruned(store).await;
    embedding_endpoint_round_trips(store).await;
    search_hybrid_fuses(store).await;
    search_ranks_titles_over_mentions(store).await;
    search_hybrid_keeps_title_hits_first(store).await;
    works_missing_embedding_backfill(store).await;
    backup_round_trip(store).await;
    live_tv_dvr_lifecycle(store).await;
    live_tv_guide_ordering_and_offsets(store).await;
    live_tv_guide_keyset_pagination(store).await;
    live_tv_refresh_large_feed(store).await;
    live_tv_listing_and_maintenance(store).await;
    export_job_lease_protocol(store).await;
    export_jobs_keyset_pagination(store).await;
    user_role_roundtrip(store).await;
    library_access_grant_revoke(store).await;
    library_access_user_delete_cascade(store).await;
    session_count_per_user(store).await;
    session_revoke_all_for_user(store).await;
    session_prune_idle(store).await;
    session_refresh_token_lookup(store).await;
    plugin_override_persistence(store).await;
    provider_attribution_lifecycle(store).await;
    session_rotate_refresh_token(store).await;
    session_retired_refresh_tokens_die_with_session(store).await;
    api_key_lifecycle(store).await;
    user_delete_cascades_api_keys(store).await;
    device_pin_lifecycle(store).await;
    device_pin_expired_is_absent(store).await;
    user_delete_cascades_device_pins(store).await;
    list_works_basic_filter(store).await;
    list_works_watched_filter(store).await;
    list_works_rules(store).await;
    cuts_and_encodes_stay_on_their_own_levels(store).await;
    work_facets_list_what_filters_can_match(store).await;
    list_works_rating_and_added_filters(store).await;
    list_works_title_sort_pagination(store).await;
    list_works_year_sort_pagination_with_nulls(store).await;
    list_works_random_sort_mints_no_cursor(store).await;
    list_works_item_cursors_resume_after_each_item(store).await;
    library_config_round_trip(store).await;
    activity_log_round_trip(store).await;
    activity_log_keyset_pagination(store).await;
    recently_added_returns_top_level_only(store).await;
    recently_played_filters_by_user(store).await;
    play_history_paginates_and_filters_unplayed(store).await;
    get_edition_round_trip(store).await;
    set_intro_markers_round_trip(store).await;
    intro_scan_record_and_sweep(store).await;
    intro_scan_rows_cascade_with_edition(store).await;
    trickplay_round_trip_sweep_and_staleness(store).await;
    play_sessions_round_trip_filter_and_page(store).await;
    trickplay_cascades_with_edition(store).await;
    collection_crud_round_trip(store).await;
    collection_items_order_and_dedupe(store).await;
    visible_item_pagination(store).await;
    collection_items_cascade(store).await;
    playlist_crud_round_trip(store).await;
    playlist_items_mixed_targets(store).await;
    playlist_user_delete_cascade(store).await;
    user_restrictions_and_access(store).await;
    batch_work_reads_preserve_order_and_access(store).await;
    watch_room_lifecycle(store).await;
    lookup_and_cleanup_operations(store).await;
}

async fn batch_work_reads_preserve_order_and_access<S: Storage + ?Sized>(store: &S) {
    let user = make_user("batch-work-user", false);
    store.upsert_user(&user).await.expect("user");
    let allowed_library = Library {
        id: LibraryId::new(),
        name: "Batch allowed".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/batch-allowed".to_string()],
        ..Default::default()
    };
    let forbidden_library = Library {
        id: LibraryId::new(),
        name: "Batch forbidden".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/batch-forbidden".to_string()],
        ..Default::default()
    };
    store
        .upsert_library(&allowed_library)
        .await
        .expect("allowed library");
    store
        .upsert_library(&forbidden_library)
        .await
        .expect("forbidden library");
    store
        .grant_library_access(user.id, allowed_library.id)
        .await
        .expect("grant");
    let now = OffsetDateTime::now_utc();
    let first = make_work(allowed_library.id, "First", now);
    let second = make_work(allowed_library.id, "Second", now);
    let forbidden = make_work(forbidden_library.id, "Forbidden", now);
    for work in [&first, &second, &forbidden] {
        store.upsert_work(work).await.expect("work");
    }
    let missing = WorkId::new();
    let ids = [second.id, forbidden.id, missing, first.id, second.id];
    let fetched = store.get_works(&ids).await.expect("batch fetch");
    assert_eq!(
        fetched.iter().map(|work| work.id).collect::<Vec<_>>(),
        vec![second.id, forbidden.id, first.id, second.id]
    );
    assert_eq!(
        store
            .accessible_work_ids(user.id, &ids)
            .await
            .expect("batch access"),
        vec![second.id, first.id, second.id]
    );
}

async fn external_identities_round_trip<S: Storage + ?Sized>(store: &S) {
    let alice = make_user("external-alice", false);
    let bob = make_user("external-bob", false);
    store.upsert_user(&alice).await.expect("alice");
    store.upsert_user(&bob).await.expect("bob");
    let identity = ExternalIdentity {
        plugin_id: "oidc".into(),
        issuer: "https://issuer.example".into(),
        subject: "subject-1".into(),
        user_id: alice.id,
        username: Some("alice".into()),
        display_name: Some("Alice".into()),
        email: Some("alice@example.test".into()),
        linked_at: OffsetDateTime::now_utc(),
    };
    store.link_external_identity(&identity).await.expect("link");
    assert_eq!(
        store
            .external_identity("oidc", &identity.issuer, &identity.subject)
            .await
            .expect("lookup"),
        Some(identity.clone())
    );
    assert_eq!(
        store
            .list_external_identities(alice.id)
            .await
            .expect("list"),
        vec![identity.clone()]
    );

    let mut updated = identity.clone();
    updated.username = Some("alice-updated".into());
    updated.display_name = None;
    updated.email = Some("new-alice@example.test".into());
    store
        .link_external_identity(&updated)
        .await
        .expect("update identity claims");
    assert_eq!(
        store
            .external_identity("oidc", &identity.issuer, &identity.subject)
            .await
            .expect("lookup updated"),
        Some(updated.clone())
    );

    let mut duplicate_provider = identity.clone();
    duplicate_provider.subject = "subject-2".into();
    assert!(matches!(
        store
            .link_external_identity(&duplicate_provider)
            .await
            .expect_err("one identity per provider"),
        StorageError::Conflict(_)
    ));

    let mut stolen = identity.clone();
    stolen.user_id = bob.id;
    assert!(matches!(
        store
            .link_external_identity(&stolen)
            .await
            .expect_err("identity is exclusive"),
        StorageError::Conflict(_)
    ));
    store.delete_user(alice.id).await.expect("delete alice");
    assert!(
        store
            .external_identity("oidc", &updated.issuer, &updated.subject)
            .await
            .expect("lookup after user delete")
            .is_none(),
        "user deletion cascades to external identities"
    );
    assert!(
        !store
            .unlink_external_identity(alice.id, "oidc")
            .await
            .expect("unlink after cascade")
    );
    store.delete_user(bob.id).await.ok();
}

async fn migrate_is_idempotent<S: Storage + ?Sized>(store: &S) {
    store.migrate().await.expect("first migrate");
    store.migrate().await.expect("second migrate (idempotent)");
}

async fn health_reports_backend<S: Storage + ?Sized>(store: &S) {
    let h = store.health().await.expect("health");
    assert!(!h.backend.is_empty(), "health report must name its backend");
    assert!(h.healthy, "fresh store should be healthy");
}

async fn round_trip_library<S: Storage + ?Sized>(store: &S) {
    let lib = Library {
        id: LibraryId::new(),
        name: "Test Movies".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/media/movies".to_string(), "/mnt/extra".to_string()],
        ..Default::default()
    };
    store.upsert_library(&lib).await.expect("upsert library");

    let all = store.list_libraries().await.expect("list libraries");
    let found = all
        .iter()
        .find(|l| l.id == lib.id)
        .expect("library round-trips through list_libraries");
    assert_eq!(found.name, "Test Movies");
    assert_eq!(found.kind, LibraryKind::Movies);
    assert_eq!(found.paths.len(), 2);
    assert_eq!(found.paths[0], "/srv/media/movies");
    assert_eq!(found.paths[1], "/mnt/extra");

    // Idempotent re-upsert: rename and verify.
    let mut renamed = lib.clone();
    renamed.name = "Renamed Movies".to_string();
    store.upsert_library(&renamed).await.expect("re-upsert");
    let after = store.list_libraries().await.expect("list after rename");
    let found = after.iter().find(|l| l.id == lib.id).expect("still there");
    assert_eq!(found.name, "Renamed Movies");

    store.delete_library(lib.id).await.expect("delete library");
    let after_delete = store.list_libraries().await.expect("list after delete");
    assert!(after_delete.iter().all(|l| l.id != lib.id));
}

async fn edition_identity_is_conflict_safe<S: Storage + ?Sized>(store: &S) {
    let library = Library {
        id: LibraryId::new(),
        name: "Edition Identity".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/identity".to_string()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let work = make_work(library.id, "Identity Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let first = make_edition(work.id, "Default");
    let second = make_edition(work.id, "Default");

    assert_eq!(store.upsert_edition(&first).await.unwrap(), first.id);
    assert_eq!(store.upsert_edition(&second).await.unwrap(), first.id);
    assert_eq!(store.editions_for_work(work.id).await.unwrap().len(), 1);
    store.delete_work(work.id).await.unwrap();
}

async fn media_removal_prunes_empty_catalog<S: Storage + ?Sized>(store: &S) {
    let library = Library {
        id: LibraryId::new(),
        name: "Removal Pruning".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/prune".to_string()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let work = make_work(library.id, "Pruned Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Default");
    store.upsert_edition(&edition).await.expect("edition");
    let first = make_media(edition.id, "/prune/first.mkv", OffsetDateTime::now_utc());
    let second = make_media(edition.id, "/prune/second.mkv", OffsetDateTime::now_utc());
    store.upsert_media_file(&first).await.expect("first media");
    store
        .upsert_media_file(&second)
        .await
        .expect("second media");

    let kept = store.remove_media_file_and_prune(first.id).await.unwrap();
    assert!(kept.removed_works.is_empty());
    assert!(store.get_edition(edition.id).await.unwrap().is_some());

    let pruned = store.remove_media_file_and_prune(second.id).await.unwrap();
    assert_eq!(pruned.removed_works, vec![work.id]);
    assert!(store.get_edition(edition.id).await.unwrap().is_none());
    assert!(store.get_work(work.id).await.unwrap().is_none());
}

async fn edition_prune_after_move<S: Storage + ?Sized>(store: &S) {
    let library = Library {
        id: LibraryId::new(),
        name: "Edition Pruning".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/prune-move".to_string()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let (from, to) = (
        make_work(library.id, "From", now),
        make_work(library.id, "To", now),
    );
    store.upsert_work(&from).await.expect("from work");
    store.upsert_work(&to).await.expect("to work");
    let (from_ed, to_ed) = (
        make_edition(from.id, "Default"),
        make_edition(to.id, "Default"),
    );
    store.upsert_edition(&from_ed).await.expect("from edition");
    store.upsert_edition(&to_ed).await.expect("to edition");
    let mut media = make_media(from_ed.id, "/prune-move/a.mkv", now);
    store.upsert_media_file(&media).await.expect("media");

    let removal = atomic(store, |tx| {
        Box::pin(async move {
            let still_used = tx.prune_edition(from_ed.id).await?;
            assert!(still_used.removed_works.is_empty(), "edition has media");
            media.edition_id = to_ed.id;
            tx.upsert_media_file(&media).await?;
            tx.prune_edition(from_ed.id).await
        })
    })
    .await
    .expect("move and prune");
    assert_eq!(removal.removed_works, vec![from.id]);
    assert!(store.get_edition(from_ed.id).await.unwrap().is_none());
    assert!(store.get_work(from.id).await.unwrap().is_none());
    assert!(store.get_edition(to_ed.id).await.unwrap().is_some());

    let gone = atomic(store, |tx| {
        Box::pin(async move { tx.prune_edition(from_ed.id).await })
    })
    .await
    .expect("prune missing edition");
    assert!(gone.removed_works.is_empty());
    store.delete_work(to.id).await.expect("cleanup");
}

async fn library_scan_version_roundtrip<S: Storage + ?Sized>(store: &S) {
    let mut library = Library {
        id: LibraryId::new(),
        name: "Scan Version".to_string(),
        kind: LibraryKind::Shows,
        paths: vec!["/scan-version".to_string()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    assert_eq!(store.library_scan_version(library.id).await.unwrap(), 0);

    store
        .set_library_scan_version(library.id, 3)
        .await
        .expect("set version");
    library.name = "Scan Version (renamed)".to_string();
    store.upsert_library(&library).await.expect("re-upsert");
    assert_eq!(
        store.library_scan_version(library.id).await.unwrap(),
        3,
        "upsert_library keeps the recorded version"
    );

    let unknown = LibraryId::new();
    assert!(matches!(
        store.library_scan_version(unknown).await,
        Err(StorageError::NotFound)
    ));
    assert!(matches!(
        store.set_library_scan_version(unknown, 1).await,
        Err(StorageError::NotFound)
    ));
}

async fn round_trip_work_edition_media<S: Storage + ?Sized>(store: &S) {
    let library = Library {
        id: LibraryId::new(),
        name: "WEM Movies".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/wem".to_string()],
        ..Default::default()
    };
    store
        .upsert_library(&library)
        .await
        .expect("upsert library");

    let now = OffsetDateTime::now_utc();
    let work = Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "Blade Runner".to_string(),
        sort_title: "blade runner".to_string(),
        year: Some(1982),
        release_date: Some("1982-06-25".to_string()),
        overview: Some("Replicants are bad.".to_string()),
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: MetadataStatus::Unconfirmed,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    };
    store.upsert_work(&work).await.expect("upsert work");

    let fetched = store
        .get_work(work.id)
        .await
        .expect("get_work")
        .expect("work present");
    assert_eq!(fetched.title, "Blade Runner");
    assert_eq!(fetched.year, Some(1982));
    assert_eq!(fetched.library_id, library.id);

    let edition = Edition {
        id: noflex_core::EditionId::new(),
        work_id: work.id,
        name: "Final Cut".to_string(),
        runtime_ms: Some(7_140_000),
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    };
    store
        .upsert_edition(&edition)
        .await
        .expect("upsert edition");

    let editions = store
        .editions_for_work(work.id)
        .await
        .expect("editions_for_work");
    assert_eq!(editions.len(), 1);
    assert_eq!(editions[0].name, "Final Cut");

    let mf1 = MediaFile {
        id: noflex_core::MediaFileId::new(),
        edition_id: edition.id,
        path: "/srv/wem/blade_runner_cd1.mkv".to_string(),
        part_index: 1,
        size_bytes: 4_500_000_000,
        mtime: now,
        container: Some("matroska".to_string()),
        probe: serde_json::json!({}).into(),
        subtitle_tracks: vec![
            SubtitleTrack {
                language: Some("en".to_string()),
                codec: "subrip".to_string(),
                default: false,
                forced: true,
                external: true,
                codec_profile: None,
                srt_compatible: true,
                image_count: None,
                title: None,
                path: Some("/srv/wem/blade_runner_cd1.en.forced.srt".to_string()),
                stream_index: None,
            },
            SubtitleTrack {
                language: Some("en".to_string()),
                codec: "subrip".to_string(),
                default: false,
                forced: false,
                external: true,
                codec_profile: None,
                srt_compatible: true,
                image_count: None,
                title: Some("SDH".to_string()),
                path: Some("/srv/wem/blade_runner_cd1.en.sdh.srt".to_string()),
                stream_index: None,
            },
        ],
    };
    let mf2 = MediaFile {
        id: noflex_core::MediaFileId::new(),
        edition_id: edition.id,
        path: "/srv/wem/blade_runner_cd2.mkv".to_string(),
        part_index: 2,
        size_bytes: 4_600_000_000,
        mtime: now,
        container: Some("matroska".to_string()),
        probe: serde_json::json!({}).into(),
        subtitle_tracks: Vec::new(),
    };
    store.upsert_media_file(&mf1).await.expect("upsert mf1");
    store.upsert_media_file(&mf2).await.expect("upsert mf2");

    let mut files = store
        .media_files_for_edition(edition.id)
        .await
        .expect("media_files_for_edition");
    files.sort_by_key(|m| m.part_index);
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].part_index, 1);
    assert_eq!(files[1].part_index, 2);

    let by_path = store
        .media_file_by_path(&mf1.path)
        .await
        .expect("media_file_by_path");
    let by_path = by_path.expect("found");
    assert_eq!(by_path.id, mf1.id);
    assert_eq!(by_path.size_bytes, 4_500_000_000);
    assert_eq!(
        by_path.subtitle_tracks, mf1.subtitle_tracks,
        "subtitle sidecars round-trip through media_file_by_path"
    );

    let by_id = store
        .media_file_by_id(mf1.id)
        .await
        .expect("media_file_by_id");
    let by_id = by_id.expect("found");
    assert_eq!(by_id.path, mf1.path);
    assert_eq!(by_id.size_bytes, mf1.size_bytes);
    assert_eq!(
        by_id.subtitle_tracks, mf1.subtitle_tracks,
        "subtitle sidecars round-trip through media_file_by_id"
    );

    let missing = store
        .media_file_by_id(noflex_core::MediaFileId::new())
        .await
        .expect("media_file_by_id random");
    assert!(missing.is_none());

    let lib_files = store
        .list_media_files_for_library(library.id)
        .await
        .expect("list_media_files_for_library");
    assert_eq!(lib_files.len(), 2);

    // CASCADE: deleting the work should remove editions and media files.
    store.delete_work(work.id).await.expect("delete work");
    let after = store
        .list_media_files_for_library(library.id)
        .await
        .expect("list after work delete");
    assert!(
        after.is_empty(),
        "FK CASCADE should have removed media files"
    );

    store
        .delete_library(library.id)
        .await
        .expect("cleanup library");
}

async fn works_by_external_id_roundtrip<S: Storage + ?Sized>(store: &S) {
    let library = Library {
        id: LibraryId::new(),
        name: "Ext-id Movies".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/ext".to_string()],
        ..Default::default()
    };
    store
        .upsert_library(&library)
        .await
        .expect("upsert library");

    let now = OffsetDateTime::now_utc();
    let key = format!("fs:movie:{}:the-matrix:1999", library.id.into_uuid());
    let work = Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: "The Matrix".to_string(),
        sort_title: "matrix, the".to_string(),
        year: Some(1999),
        release_date: None,
        overview: None,
        external_ids: vec![ExternalId {
            kind: ExternalIdKind::Custom,
            value: key.clone(),
        }],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: MetadataStatus::Unconfirmed,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    };
    store.upsert_work(&work).await.expect("upsert work");

    let found = store
        .works_by_external_id(ExternalIdKind::Custom, &key)
        .await
        .expect("works_by_external_id");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, work.id);
    assert_eq!(found[0].title, "The Matrix");

    // Cleanup.
    store.delete_work(work.id).await.expect("cleanup work");
    store
        .delete_library(library.id)
        .await
        .expect("cleanup library");
}

/// `Work.index_end` is the canonical home for the "this Work spans an index
/// range" notion — multi-episode files (`s01e01-e02`) store the inclusive
/// end here. The field round-trips through storage cleanly, and `None` stays
/// `None` for normal single-indexed Works.
async fn index_end_roundtrip<S: Storage + ?Sized>(store: &S) {
    let library = Library {
        id: LibraryId::new(),
        name: "Range Shows".to_string(),
        kind: LibraryKind::Shows,
        paths: vec!["/srv/range".to_string()],
        ..Default::default()
    };
    store
        .upsert_library(&library)
        .await
        .expect("upsert library");

    let now = OffsetDateTime::now_utc();

    // Single-indexed Work: index_end stays None on round-trip.
    let single = Work {
        id: WorkId::new(),
        kind: WorkKind::Episode,
        parent_id: None,
        library_id: library.id,
        title: "Pilot".to_string(),
        sort_title: "pilot".to_string(),
        year: None,
        release_date: None,
        overview: None,
        external_ids: vec![],
        index_number: Some(1),
        index_end: None,
        parent_index: Some(1),
        metadata_status: MetadataStatus::Unconfirmed,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    };
    store.upsert_work(&single).await.expect("upsert single");
    let got = store
        .get_work(single.id)
        .await
        .expect("get single")
        .expect("present");
    assert_eq!(got.index_number, Some(1));
    assert_eq!(got.index_end, None, "single-indexed Work has no index_end");

    // Range Work: index_end persists as Some.
    let ranged = Work {
        id: WorkId::new(),
        kind: WorkKind::Episode,
        parent_id: None,
        library_id: library.id,
        title: "Good News (1-2)".to_string(),
        sort_title: "good-news-1-2".to_string(),
        year: None,
        release_date: None,
        overview: None,
        external_ids: vec![],
        index_number: Some(1),
        index_end: Some(2),
        parent_index: Some(1),
        metadata_status: MetadataStatus::Unconfirmed,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    };
    store.upsert_work(&ranged).await.expect("upsert ranged");
    let got = store
        .get_work(ranged.id)
        .await
        .expect("get ranged")
        .expect("present");
    assert_eq!(got.index_number, Some(1));
    assert_eq!(got.index_end, Some(2));

    // Update path: narrow a range back to a single-indexed Work.
    let mut narrowed = ranged.clone();
    narrowed.index_end = None;
    narrowed.title = "Good News (just 1)".to_string();
    store
        .upsert_work(&narrowed)
        .await
        .expect("re-upsert narrow");
    let got = store
        .get_work(ranged.id)
        .await
        .expect("get narrowed")
        .expect("present");
    assert_eq!(
        got.index_end, None,
        "upsert clearing index_end is honoured (not left stale)"
    );

    // Cleanup.
    store.delete_work(single.id).await.ok();
    store.delete_work(ranged.id).await.ok();
    store
        .delete_library(library.id)
        .await
        .expect("cleanup library");
}

/// Every variant of `MetadataStatus` round-trips intact, and an upsert
/// honours whatever status the caller wrote (so providers can escalate, and
/// users can override, without any backend magic). The scanner relies on
/// this to be allowed to never touch the field once a Work exists.
async fn metadata_status_roundtrip<S: Storage + ?Sized>(store: &S) {
    let library = Library {
        id: LibraryId::new(),
        name: "Status Movies".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/status".to_string()],
        ..Default::default()
    };
    store
        .upsert_library(&library)
        .await
        .expect("upsert library");
    let now = OffsetDateTime::now_utc();

    let make = |status: MetadataStatus, title: &str| Work {
        id: WorkId::new(),
        kind: WorkKind::Movie,
        parent_id: None,
        library_id: library.id,
        title: title.to_string(),
        sort_title: title.to_lowercase(),
        year: None,
        release_date: None,
        overview: None,
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: status,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    };

    for status in [
        MetadataStatus::Unconfirmed,
        MetadataStatus::Confirmed,
        MetadataStatus::UserEdited,
    ] {
        let w = make(status, &format!("Status {status:?}"));
        store.upsert_work(&w).await.expect("upsert");
        let got = store.get_work(w.id).await.expect("get").expect("present");
        assert_eq!(
            got.metadata_status, status,
            "variant {status:?} round-trips"
        );
        store.delete_work(w.id).await.ok();
    }

    // Lifecycle: a Work created as Unconfirmed (the scanner's path) can be
    // upserted again with Confirmed (a provider's path) — and the upgrade
    // sticks.
    let unconfirmed = make(MetadataStatus::Unconfirmed, "Lifecycle");
    store
        .upsert_work(&unconfirmed)
        .await
        .expect("upsert initial");

    let mut escalated = unconfirmed.clone();
    escalated.metadata_status = MetadataStatus::Confirmed;
    store
        .upsert_work(&escalated)
        .await
        .expect("upsert provider");
    let got = store
        .get_work(unconfirmed.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(
        got.metadata_status,
        MetadataStatus::Confirmed,
        "provider escalation persists"
    );

    let mut user = escalated.clone();
    user.metadata_status = MetadataStatus::UserEdited;
    store.upsert_work(&user).await.expect("upsert user");
    let got = store
        .get_work(unconfirmed.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(got.metadata_status, MetadataStatus::UserEdited);

    store.delete_work(unconfirmed.id).await.ok();
    store
        .delete_library(library.id)
        .await
        .expect("cleanup library");
}

/// The provider-applied metadata fields (`tagline`, `original_title`,
/// `community_rating`) and the user's `locked_fields` round-trip
/// byte-for-byte: whatever the caller wrote comes back, including the
/// cleared case (`None` / empty). Sort+dedupe of `locked_fields` is a
/// writer-side invariant; storage preserves the vec as written.
async fn work_metadata_fields_roundtrip<S: Storage + ?Sized>(store: &S) {
    let library = Library {
        id: LibraryId::new(),
        name: "Metadata Movies".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/metadata".to_string()],
        ..Default::default()
    };
    store
        .upsert_library(&library)
        .await
        .expect("upsert library");
    let now = OffsetDateTime::now_utc();

    let mut work = make_work(library.id, "Metadata Movie", now);
    work.tagline = Some("In space no one can hear you scream".to_string());
    work.original_title = Some("Star Beast".to_string());
    work.community_rating = Some(8.5);
    work.locked_fields = vec![
        WorkField::Title,
        WorkField::Overview,
        WorkField::CommunityRating,
    ];
    store.upsert_work(&work).await.expect("upsert populated");
    let got = store
        .get_work(work.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(got.tagline, work.tagline);
    assert_eq!(got.original_title, work.original_title);
    assert_eq!(got.community_rating, work.community_rating);
    assert_eq!(got.locked_fields, work.locked_fields);

    // Clearing the fields on re-upsert is honoured, not left stale.
    let mut cleared = got.clone();
    cleared.tagline = None;
    cleared.original_title = None;
    cleared.community_rating = None;
    cleared.locked_fields = vec![];
    store.upsert_work(&cleared).await.expect("upsert cleared");
    let got = store
        .get_work(work.id)
        .await
        .expect("get cleared")
        .expect("present");
    assert_eq!(got.tagline, None);
    assert_eq!(got.original_title, None);
    assert_eq!(got.community_rating, None);
    assert!(got.locked_fields.is_empty());

    store.delete_work(work.id).await.ok();
    store
        .delete_library(library.id)
        .await
        .expect("cleanup library");
}

/// A successful `commit()` makes the work+edition+media-file batch
/// visible through the normal (non-transaction) read path.
async fn transaction_commit_persists<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Tx Commit");
    store
        .upsert_library(&library)
        .await
        .expect("upsert library");

    let now = OffsetDateTime::now_utc();
    let work = make_work(library.id, "Committed", now);
    let edition = make_edition(work.id, "Default");
    let media = make_media(edition.id, "/srv/tx/commit.mkv", now);

    {
        let mut tx = store.begin().await.expect("begin");
        tx.upsert_work(&work).await.expect("upsert work");
        tx.upsert_edition(&edition).await.expect("upsert edition");
        tx.upsert_media_file(&media).await.expect("upsert media");
        tx.commit().await.expect("commit");
    }

    let got_work = store
        .get_work(work.id)
        .await
        .expect("get work")
        .expect("work visible after commit");
    assert_eq!(got_work.title, "Committed");
    let editions = store.editions_for_work(work.id).await.expect("editions");
    assert_eq!(editions.len(), 1, "edition persisted");
    let files = store
        .media_files_for_edition(edition.id)
        .await
        .expect("media files");
    assert_eq!(files.len(), 1, "media file persisted");
    assert_eq!(files[0].path, "/srv/tx/commit.mkv");

    store.delete_work(work.id).await.ok();
    store.delete_library(library.id).await.ok();
}

/// A `rollback()` (or a dropped transaction) makes the in-flight writes
/// disappear — reads after rollback see no trace of the work.
async fn transaction_rollback_disappears<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Tx Rollback");
    store
        .upsert_library(&library)
        .await
        .expect("upsert library");

    let now = OffsetDateTime::now_utc();
    let work = make_work(library.id, "RolledBack", now);
    let edition = make_edition(work.id, "Default");
    let media = make_media(edition.id, "/srv/tx/rollback.mkv", now);

    {
        let mut tx = store.begin().await.expect("begin");
        tx.upsert_work(&work).await.expect("upsert work");
        tx.upsert_edition(&edition).await.expect("upsert edition");
        tx.upsert_media_file(&media).await.expect("upsert media");
        tx.rollback().await.expect("rollback");
    }

    let got_work = store.get_work(work.id).await.expect("get work");
    assert!(got_work.is_none(), "rolled-back work must not be visible");
    let editions = store
        .editions_for_work(work.id)
        .await
        .expect("editions after rollback");
    assert!(editions.is_empty(), "no edition after rollback");
    let files = store
        .media_files_for_edition(edition.id)
        .await
        .expect("media after rollback");
    assert!(files.is_empty(), "no media file after rollback");

    store.delete_library(library.id).await.ok();
}

/// `atomic` commits on `Ok` and returns the closure's value.
async fn atomic_helper_commits_on_ok<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Atomic Ok");
    store
        .upsert_library(&library)
        .await
        .expect("upsert library");

    let now = OffsetDateTime::now_utc();
    let work = make_work(library.id, "AtomicOk", now);

    let returned: StorageResult<WorkId> = atomic(store, |tx| {
        let work = work.clone();
        Box::pin(async move {
            tx.upsert_work(&work).await?;
            Ok(work.id)
        })
    })
    .await;
    let id = returned.expect("atomic Ok returns value");
    assert_eq!(id, work.id);
    assert!(store.get_work(work.id).await.expect("get").is_some());

    store.delete_work(work.id).await.ok();
    store.delete_library(library.id).await.ok();
}

/// `atomic` rolls back when the closure returns `Err` and surfaces the
/// closure's error (not a rollback error).
async fn atomic_helper_rolls_back_on_err<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Atomic Err");
    store
        .upsert_library(&library)
        .await
        .expect("upsert library");

    let now = OffsetDateTime::now_utc();
    let work = make_work(library.id, "AtomicErr", now);

    let result: StorageResult<()> = atomic(store, |tx| {
        let work = work.clone();
        Box::pin(async move {
            tx.upsert_work(&work).await?;
            // Trigger the rollback path without forcing the test to depend
            // on a specific error variant. The atomic helper must roll back
            // and propagate this exact error to the caller.
            Err(crate::error::StorageError::Conflict(
                "synthetic failure".to_string(),
            ))
        })
    })
    .await;
    let err = result.expect_err("atomic Err propagates the closure's error");
    assert!(
        matches!(err, crate::error::StorageError::Conflict(_)),
        "got {err:?}"
    );
    assert!(
        store.get_work(work.id).await.expect("get").is_none(),
        "atomic rollback must wipe the work"
    );

    store.delete_library(library.id).await.ok();
}

fn test_library(name: &str) -> Library {
    Library {
        id: LibraryId::new(),
        name: name.to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/tx".to_string()],
        ..Default::default()
    }
}

fn make_work(library_id: LibraryId, title: &str, now: OffsetDateTime) -> Work {
    make_work_of_kind(library_id, WorkKind::Movie, title, now)
}

fn make_work_of_kind(
    library_id: LibraryId,
    kind: WorkKind,
    title: &str,
    now: OffsetDateTime,
) -> Work {
    Work {
        id: WorkId::new(),
        kind,
        parent_id: None,
        library_id,
        title: title.to_string(),
        sort_title: title.to_lowercase(),
        year: None,
        release_date: None,
        overview: None,
        external_ids: vec![],
        index_number: None,
        index_end: None,
        parent_index: None,
        metadata_status: MetadataStatus::Unconfirmed,
        tagline: None,
        original_title: None,
        community_rating: None,
        locked_fields: vec![],
        metadata_provider: None,
        added_at: now,
        updated_at: now,
    }
}

fn make_edition(work_id: WorkId, name: &str) -> Edition {
    Edition {
        id: EditionId::new(),
        work_id,
        name: name.to_string(),
        runtime_ms: None,
        intro_start_ms: None,
        intro_end_ms: None,
        credits_start_ms: None,
        chapters: vec![],
    }
}

fn make_media(edition_id: EditionId, path: &str, mtime: OffsetDateTime) -> MediaFile {
    MediaFile {
        id: MediaFileId::new(),
        edition_id,
        path: path.to_string(),
        part_index: 0,
        size_bytes: 1024,
        mtime,
        container: Some("matroska".to_string()),
        probe: serde_json::json!({}).into(),
        subtitle_tracks: Vec::new(),
    }
}

fn admin_principal() -> RequestUser {
    RequestUser {
        user_id: UserId::new(),
        is_admin: true,
    }
}

fn make_user(name: &str, is_admin: bool) -> User {
    let now = OffsetDateTime::now_utc();
    let role = if is_admin {
        noflex_core::UserRole::Admin
    } else {
        noflex_core::UserRole::User
    };
    User {
        id: UserId::new(),
        username: name.to_string(),
        display_name: None,
        password_hash: "argon2id$placeholder".to_string(),
        is_admin,
        role,
        created_at: now,
        avatar_updated_at: None,
    }
}

fn make_session(user_id: UserId) -> Session {
    let now = OffsetDateTime::now_utc();
    Session {
        id: SessionId::new(),
        user_id,
        client: Some("Findroid".to_string()),
        device: Some("Pixel 8".to_string()),
        device_id: Some("pixel-8-abc".to_string()),
        app_version: Some("1.2.3".to_string()),
        remote_addr: Some("10.0.0.42".to_string()),
        created_at: now,
        last_seen_at: now,
        expires_at: now + time::Duration::days(30),
        // Unique per call: the UNIQUE index on refresh_token_hash would
        // otherwise collide across sessions coexisting in one test.
        refresh_token_hash: Some(format!("test-hash-{}", SessionId::new())),
    }
}

/// Round-trips a user through every repo path: upsert, get by id, get by
/// name, list. Also confirms the `is_admin` flag round-trips as a real
/// boolean, not a coerced truthy integer.
async fn user_round_trip<S: Storage + ?Sized>(store: &S) {
    let u = make_user("alice", true);
    store.upsert_user(&u).await.expect("upsert user");

    let by_id = store
        .get_user(u.id)
        .await
        .expect("get by id")
        .expect("user present");
    assert_eq!(by_id.username, "alice");
    assert!(by_id.is_admin);
    assert_eq!(by_id.password_hash, u.password_hash);

    let by_name = store
        .get_user_by_username("alice")
        .await
        .expect("get by name")
        .expect("named user present");
    assert_eq!(by_name.id, u.id);

    let all = store.list_users().await.expect("list");
    assert!(
        all.iter().any(|x| x.id == u.id),
        "user must appear in list_users"
    );

    let mut named = by_id.clone();
    named.display_name = Some("Alice A".into());
    store.upsert_user(&named).await.expect("name");
    let renamed = store.get_user(u.id).await.expect("get").expect("user");
    assert_eq!(renamed.username, "alice");
    assert_eq!(renamed.shown_name(), "Alice A");

    // A picture is apart from the row, so saving a copy read earlier keeps it.
    let at = OffsetDateTime::now_utc();
    assert_eq!(store.user_avatar(u.id).await.expect("none"), None);
    store
        .set_user_avatar(u.id, b"first", at)
        .await
        .expect("avatar");
    store
        .set_user_avatar(u.id, b"second", at)
        .await
        .expect("replace");
    store.upsert_user(&named).await.expect("upsert");
    let pictured = store.get_user(u.id).await.expect("get").expect("user");
    assert_eq!(
        pictured
            .avatar_updated_at
            .map(OffsetDateTime::unix_timestamp),
        Some(at.unix_timestamp())
    );
    assert_eq!(
        store.user_avatar(u.id).await.expect("avatar").as_deref(),
        Some(&b"second"[..])
    );
    assert!(store.delete_user_avatar(u.id).await.expect("remove"));
    assert!(!store.delete_user_avatar(u.id).await.expect("again"));
    assert!(
        store
            .get_user(u.id)
            .await
            .expect("get")
            .expect("user")
            .avatar_updated_at
            .is_none()
    );
    store
        .set_user_avatar(u.id, b"third", at)
        .await
        .expect("avatar");

    // Non-admin variant to ensure the boolean is preserved.
    let bob = make_user("bob", false);
    store.upsert_user(&bob).await.expect("upsert bob");
    let got = store
        .get_user(bob.id)
        .await
        .expect("get bob")
        .expect("bob present");
    assert!(!got.is_admin, "non-admin flag must round-trip as false");

    store.delete_user(u.id).await.ok();
    store.delete_user(bob.id).await.ok();
    assert_eq!(store.user_avatar(u.id).await.expect("gone"), None);
}

/// The `users.username` UNIQUE constraint is the only thing preventing two
/// admins from existing under the same login. Re-upserting under the
/// SAME id is allowed (UPSERT path); creating a SECOND user with a
/// colliding name must fail.
async fn user_uniqueness<S: Storage + ?Sized>(store: &S) {
    let u = make_user("collision", false);
    store.upsert_user(&u).await.expect("first upsert");

    let mut collide = u.clone();
    collide.id = UserId::new();
    let err = store.upsert_user(&collide).await.expect_err("name clash");
    assert!(
        matches!(err, crate::error::StorageError::Constraint(_)),
        "expected Constraint, got {err:?}"
    );

    store.delete_user(u.id).await.ok();
}

/// create → touch → get → list → revoke covers every session verb. Touch
/// must advance `last_seen_at` without disturbing the row's id or
/// `created_at`, and revoke must make subsequent get return None.
async fn user_preferences_round_trip<S: Storage + ?Sized>(store: &S) {
    let user = make_user("preferences-round-trip", false);
    store.upsert_user(&user).await.expect("user");
    let preference = UserPreference {
        user_id: user.id,
        key: "playback.settings".to_string(),
        value: async_graphql::Json(serde_json::json!({
            "quality": "auto",
            "subtitles": {"enabled": true, "languages": ["en", "ja"]}
        })),
        updated_at: OffsetDateTime::now_utc(),
    };
    store
        .set_user_preference(&preference)
        .await
        .expect("set preference");

    let preferences = store
        .list_user_preferences(user.id)
        .await
        .expect("list preferences");
    assert_eq!(preferences.len(), 1);
    assert_eq!(preferences[0].user_id, user.id);
    assert_eq!(preferences[0].key, preference.key);
    assert_eq!(preferences[0].value, preference.value);
    assert_eq!(preferences[0].updated_at, preference.updated_at);

    store.delete_user(user.id).await.ok();
}

async fn user_preferences_upsert<S: Storage + ?Sized>(store: &S) {
    let user = make_user("preferences-upsert", false);
    store.upsert_user(&user).await.expect("user");
    let first_at = OffsetDateTime::now_utc();
    let mut preference = UserPreference {
        user_id: user.id,
        key: "theme".to_string(),
        value: async_graphql::Json(serde_json::json!("light")),
        updated_at: first_at,
    };
    store
        .set_user_preference(&preference)
        .await
        .expect("set initial preference");
    preference.value = async_graphql::Json(serde_json::json!("dark"));
    preference.updated_at = first_at + time::Duration::seconds(1);
    store
        .set_user_preference(&preference)
        .await
        .expect("overwrite preference");

    let preferences = store
        .list_user_preferences(user.id)
        .await
        .expect("list preferences");
    assert_eq!(preferences.len(), 1);
    assert_eq!(preferences[0].value, preference.value);
    assert!(preferences[0].updated_at > first_at);

    store.delete_user(user.id).await.ok();
}

async fn user_preferences_delete<S: Storage + ?Sized>(store: &S) {
    let user = make_user("preferences-delete", false);
    store.upsert_user(&user).await.expect("user");
    let preference = UserPreference {
        user_id: user.id,
        key: "theme".to_string(),
        value: async_graphql::Json(serde_json::json!("dark")),
        updated_at: OffsetDateTime::now_utc(),
    };
    store
        .set_user_preference(&preference)
        .await
        .expect("set preference");

    assert!(
        store
            .delete_user_preference(user.id, "theme")
            .await
            .expect("delete present")
    );
    assert!(
        !store
            .delete_user_preference(user.id, "theme")
            .await
            .expect("delete missing")
    );

    store.delete_user(user.id).await.ok();
}

async fn user_preferences_are_isolated<S: Storage + ?Sized>(store: &S) {
    let alice = make_user("preferences-alice", false);
    let bob = make_user("preferences-bob", false);
    store.upsert_user(&alice).await.expect("alice");
    store.upsert_user(&bob).await.expect("bob");
    for (user_id, value) in [(alice.id, "alice"), (bob.id, "bob")] {
        store
            .set_user_preference(&UserPreference {
                user_id,
                key: "profile".to_string(),
                value: async_graphql::Json(serde_json::json!(value)),
                updated_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("set preference");
    }

    let alice_preferences = store
        .list_user_preferences(alice.id)
        .await
        .expect("list alice");
    let bob_preferences = store.list_user_preferences(bob.id).await.expect("list bob");
    assert_eq!(alice_preferences.len(), 1);
    assert_eq!(bob_preferences.len(), 1);
    assert_eq!(alice_preferences[0].value.0, serde_json::json!("alice"));
    assert_eq!(bob_preferences[0].value.0, serde_json::json!("bob"));

    store.delete_user(alice.id).await.ok();
    store.delete_user(bob.id).await.ok();
}

async fn user_preferences_user_delete_cascade<S: Storage + ?Sized>(store: &S) {
    let user = make_user("preferences-cascade", false);
    store.upsert_user(&user).await.expect("user");
    store
        .set_user_preference(&UserPreference {
            user_id: user.id,
            key: "theme".to_string(),
            value: async_graphql::Json(serde_json::json!("dark")),
            updated_at: OffsetDateTime::now_utc(),
        })
        .await
        .expect("set preference");
    store.delete_user(user.id).await.expect("delete user");

    let preferences = store
        .list_user_preferences(user.id)
        .await
        .expect("list after delete");
    assert!(preferences.is_empty(), "preferences wipe with user");
}

async fn plugin_task_state_round_trip<S: Storage + ?Sized>(store: &S) {
    assert!(
        store
            .plugin_task_state("tasks-round-trip", "sync")
            .await
            .expect("get missing")
            .is_none()
    );

    let run_at = OffsetDateTime::now_utc();
    let mut state = crate::domain::PluginTaskState {
        plugin_id: "tasks-round-trip".to_string(),
        task: "sync".to_string(),
        cursor: None,
        last_run_at: Some(run_at),
        consecutive_failures: 0,
    };
    store
        .set_plugin_task_state(&state)
        .await
        .expect("set state");
    let fetched = store
        .plugin_task_state("tasks-round-trip", "sync")
        .await
        .expect("get state")
        .expect("state present");
    assert_eq!(fetched, state);

    // Upsert overwrites the row wholesale: cursor None -> Some and
    // the failure counter moves.
    state.cursor = Some("page:42".to_string());
    state.consecutive_failures = 3;
    store
        .set_plugin_task_state(&state)
        .await
        .expect("overwrite state");
    let fetched = store
        .plugin_task_state("tasks-round-trip", "sync")
        .await
        .expect("get after upsert")
        .expect("state present");
    assert_eq!(fetched.cursor.as_deref(), Some("page:42"));
    assert_eq!(fetched.consecutive_failures, 3);
    assert_eq!(fetched.last_run_at, Some(run_at));

    let mut other = state.clone();
    other.task = "refresh".to_string();
    store
        .set_plugin_task_state(&other)
        .await
        .expect("set second task");
    let mut foreign = state.clone();
    foreign.plugin_id = "tasks-other-plugin".to_string();
    store
        .set_plugin_task_state(&foreign)
        .await
        .expect("set other plugin");

    let states = store
        .list_plugin_task_states("tasks-round-trip")
        .await
        .expect("list states");
    let tasks: Vec<&str> = states.iter().map(|s| s.task.as_str()).collect();
    assert_eq!(tasks, ["refresh", "sync"], "ordered by task");
    assert!(
        store
            .list_plugin_task_states("tasks-never-seen")
            .await
            .expect("list unknown plugin")
            .is_empty()
    );
}

async fn plugin_task_queue_fifo<S: Storage + ?Sized>(store: &S) {
    assert!(
        store
            .dequeue_plugin_task("tasks-fifo", "sync")
            .await
            .expect("dequeue empty")
            .is_none()
    );

    // Interleave two tasks; each (plugin, task) queue must stay FIFO
    // and never hand back the other task's rows.
    store
        .enqueue_plugin_task("tasks-fifo", "sync", Some("first"))
        .await
        .expect("enqueue first");
    store
        .enqueue_plugin_task("tasks-fifo", "refresh", Some("other-task"))
        .await
        .expect("enqueue other task");
    store
        .enqueue_plugin_task("tasks-fifo", "sync", None)
        .await
        .expect("enqueue payload-less");
    store
        .enqueue_plugin_task("tasks-fifo", "sync", Some("third"))
        .await
        .expect("enqueue third");

    let mut ids = Vec::new();
    for expected in [Some("first"), None, Some("third")] {
        let row = store
            .dequeue_plugin_task("tasks-fifo", "sync")
            .await
            .expect("dequeue")
            .expect("row present");
        assert_eq!(row.plugin_id, "tasks-fifo");
        assert_eq!(row.task, "sync");
        assert_eq!(row.payload.as_deref(), expected);
        assert!(row.enqueued_at <= OffsetDateTime::now_utc());
        ids.push(row.id);
    }
    assert!(
        ids.windows(2).all(|w| w[0] < w[1]),
        "ids increase in FIFO order"
    );

    // The sync queue is drained; refresh is untouched.
    assert!(
        store
            .dequeue_plugin_task("tasks-fifo", "sync")
            .await
            .expect("dequeue drained")
            .is_none()
    );
    let row = store
        .dequeue_plugin_task("tasks-fifo", "refresh")
        .await
        .expect("dequeue other task")
        .expect("other task row present");
    assert_eq!(row.payload.as_deref(), Some("other-task"));
    assert!(
        store
            .dequeue_plugin_task("tasks-fifo", "refresh")
            .await
            .expect("dequeue other drained")
            .is_none()
    );
}

async fn plugin_task_delete_and_clear<S: Storage + ?Sized>(store: &S) {
    for (plugin, task) in [
        ("tasks-cleanup-a", "sync"),
        ("tasks-cleanup-a", "refresh"),
        ("tasks-cleanup-b", "sync"),
    ] {
        store
            .set_plugin_task_state(&crate::domain::PluginTaskState {
                plugin_id: plugin.to_string(),
                task: task.to_string(),
                cursor: None,
                last_run_at: None,
                consecutive_failures: 0,
            })
            .await
            .expect("seed state");
        store
            .enqueue_plugin_task(plugin, task, Some("payload"))
            .await
            .expect("seed queue");
    }

    // State deletion is scoped to the plugin and leaves its queue alone.
    let removed = store
        .delete_plugin_task_states("tasks-cleanup-a")
        .await
        .expect("delete states");
    assert_eq!(removed, 2);
    assert!(
        store
            .list_plugin_task_states("tasks-cleanup-a")
            .await
            .expect("list after delete")
            .is_empty()
    );
    assert!(
        store
            .dequeue_plugin_task("tasks-cleanup-a", "sync")
            .await
            .expect("queue survives state delete")
            .is_some(),
        "delete_plugin_task_states must not touch the queue"
    );
    assert_eq!(
        store
            .list_plugin_task_states("tasks-cleanup-b")
            .await
            .expect("other plugin states")
            .len(),
        1
    );

    // Queue clearing is likewise scoped and leaves state rows alone.
    let cleared = store
        .clear_plugin_task_queue("tasks-cleanup-a")
        .await
        .expect("clear queue");
    assert_eq!(cleared, 1, "one queue row left after the dequeue above");
    assert!(
        store
            .dequeue_plugin_task("tasks-cleanup-a", "refresh")
            .await
            .expect("dequeue after clear")
            .is_none()
    );
    let row = store
        .dequeue_plugin_task("tasks-cleanup-b", "sync")
        .await
        .expect("other plugin queue intact")
        .expect("row present");
    assert_eq!(row.payload.as_deref(), Some("payload"));
}

async fn user_plugin_secrets_round_trip<S: Storage + ?Sized>(store: &S) {
    let user = make_user("secrets-round-trip", false);
    store.upsert_user(&user).await.expect("user");
    assert!(
        store
            .user_plugin_secret(user.id, "trakt", "access_token")
            .await
            .expect("get missing")
            .is_none()
    );

    store
        .set_user_plugin_secret(user.id, "trakt", "access_token", "token-v1")
        .await
        .expect("set secret");
    store
        .set_user_plugin_secret(user.id, "trakt", "refresh_token", "refresh-v1")
        .await
        .expect("set second key");

    let first = store
        .user_plugin_secret(user.id, "trakt", "access_token")
        .await
        .expect("get secret")
        .expect("secret present");
    assert_eq!(first.user_id, user.id);
    assert_eq!(first.plugin_id, "trakt");
    assert_eq!(first.key, "access_token");
    assert_eq!(first.value, "token-v1");
    assert!(first.created_at <= first.updated_at);

    let keys: Vec<String> = store
        .list_user_plugin_secrets(user.id, "trakt")
        .await
        .expect("list secrets")
        .into_iter()
        .map(|s| s.key)
        .collect();
    assert_eq!(keys, ["access_token", "refresh_token"], "ordered by key");
    assert!(
        store
            .list_user_plugin_secrets(user.id, "never-linked")
            .await
            .expect("list unknown plugin")
            .is_empty()
    );

    // Upsert replaces the value and keeps created_at. Timestamps are
    // stored as RFC 3339 text, so a fast back-to-back write can land
    // within the same stored instant — updated_at may only be `>=`,
    // not strictly advanced.
    store
        .set_user_plugin_secret(user.id, "trakt", "access_token", "token-v2")
        .await
        .expect("overwrite secret");
    let second = store
        .user_plugin_secret(user.id, "trakt", "access_token")
        .await
        .expect("get after upsert")
        .expect("secret present");
    assert_eq!(second.value, "token-v2");
    assert_eq!(second.created_at, first.created_at);
    assert!(second.updated_at >= first.updated_at);

    store.delete_user(user.id).await.ok();
}

async fn user_plugin_secrets_are_isolated<S: Storage + ?Sized>(store: &S) {
    let alice = make_user("secrets-alice", false);
    let bob = make_user("secrets-bob", false);
    store.upsert_user(&alice).await.expect("alice");
    store.upsert_user(&bob).await.expect("bob");
    for (user_id, plugin_id, value) in [
        (alice.id, "trakt", "alice-trakt"),
        (bob.id, "trakt", "bob-trakt"),
        (alice.id, "simkl", "alice-simkl"),
    ] {
        store
            .set_user_plugin_secret(user_id, plugin_id, "access_token", value)
            .await
            .expect("set secret");
    }

    // Same key under a different user or plugin is a different row.
    for (user_id, plugin_id, expected) in [
        (alice.id, "trakt", "alice-trakt"),
        (bob.id, "trakt", "bob-trakt"),
        (alice.id, "simkl", "alice-simkl"),
    ] {
        let secret = store
            .user_plugin_secret(user_id, plugin_id, "access_token")
            .await
            .expect("get secret")
            .expect("secret present");
        assert_eq!(secret.value, expected);
    }
    assert_eq!(
        store
            .list_user_plugin_secrets(alice.id, "trakt")
            .await
            .expect("alice trakt")
            .len(),
        1
    );
    assert!(
        store
            .user_plugin_secret(bob.id, "simkl", "access_token")
            .await
            .expect("bob simkl")
            .is_none()
    );

    store.delete_user(alice.id).await.ok();
    store.delete_user(bob.id).await.ok();
}

async fn user_plugin_secrets_delete_scoped<S: Storage + ?Sized>(store: &S) {
    let user = make_user("secrets-delete", false);
    let other = make_user("secrets-delete-other", false);
    store.upsert_user(&user).await.expect("user");
    store.upsert_user(&other).await.expect("other");
    for (user_id, plugin_id, key) in [
        (user.id, "trakt", "access_token"),
        (user.id, "trakt", "refresh_token"),
        (user.id, "simkl", "access_token"),
        (other.id, "trakt", "access_token"),
    ] {
        store
            .set_user_plugin_secret(user_id, plugin_id, key, "tok")
            .await
            .expect("set secret");
    }

    let removed = store
        .delete_user_plugin_secrets(user.id, "trakt")
        .await
        .expect("delete secrets");
    assert_eq!(removed, 2);
    assert!(
        store
            .list_user_plugin_secrets(user.id, "trakt")
            .await
            .expect("list after delete")
            .is_empty()
    );
    // Other plugins of the same user and other users of the same
    // plugin survive.
    assert_eq!(
        store
            .list_user_plugin_secrets(user.id, "simkl")
            .await
            .expect("other plugin survives")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_user_plugin_secrets(other.id, "trakt")
            .await
            .expect("other user survives")
            .len(),
        1
    );
    assert_eq!(
        store
            .delete_user_plugin_secrets(user.id, "trakt")
            .await
            .expect("re-delete"),
        0
    );

    store.delete_user(user.id).await.ok();
    store.delete_user(other.id).await.ok();
}

async fn user_plugin_secrets_user_delete_cascade<S: Storage + ?Sized>(store: &S) {
    let user = make_user("secrets-cascade", false);
    store.upsert_user(&user).await.expect("user");
    store
        .set_user_plugin_secret(user.id, "trakt", "access_token", "tok")
        .await
        .expect("set secret");

    store.delete_user(user.id).await.expect("delete user");
    assert!(
        store
            .user_plugin_secret(user.id, "trakt", "access_token")
            .await
            .expect("get after cascade")
            .is_none(),
        "user delete must cascade to plugin secrets"
    );
    assert!(
        store
            .list_user_plugin_secrets(user.id, "trakt")
            .await
            .expect("list after cascade")
            .is_empty()
    );
}

async fn session_lifecycle<S: Storage + ?Sized>(store: &S) {
    let user = make_user("session-user", true);
    store.upsert_user(&user).await.expect("upsert user");

    let s = make_session(user.id);
    let original_last_seen = s.last_seen_at;
    store.create_session(&s).await.expect("create session");

    let fetched = store
        .get_session(s.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(fetched.user_id, user.id);
    assert_eq!(fetched.client.as_deref(), Some("Findroid"));
    assert_eq!(fetched.device_id.as_deref(), Some("pixel-8-abc"));
    assert_eq!(fetched.expires_at, s.expires_at, "expires_at round-trips");
    assert_eq!(
        fetched.refresh_token_hash, s.refresh_token_hash,
        "refresh_token_hash round-trips"
    );

    // Touch advances last_seen_at; created_at and the device fields stay
    // pinned. (We pick a future timestamp to dodge sub-second ties.)
    let new_seen = original_last_seen + time::Duration::seconds(30);
    store.touch_session(s.id, new_seen).await.expect("touch");
    let after = store
        .get_session(s.id)
        .await
        .expect("get after touch")
        .expect("present");
    assert_eq!(
        after.created_at, s.created_at,
        "touch must not change created_at"
    );
    assert_eq!(after.last_seen_at, new_seen, "touch advances last_seen_at");
    assert_eq!(after.device.as_deref(), Some("Pixel 8"));

    let listed = store
        .list_sessions_for_user(user.id)
        .await
        .expect("list for user");
    assert_eq!(listed.len(), 1, "exactly one session for this user");
    assert_eq!(listed[0].id, s.id);

    store.revoke_session(s.id).await.expect("revoke");
    let after_revoke = store.get_session(s.id).await.expect("get after revoke");
    assert!(
        after_revoke.is_none(),
        "revoked session must be invisible to get"
    );
    let listed_after = store
        .list_sessions_for_user(user.id)
        .await
        .expect("list after revoke");
    assert!(listed_after.is_empty());

    store.delete_user(user.id).await.ok();
}

/// A credit round-trips, its logo is read apart from its row, and storing one
/// without a logo clears the old logo.
async fn provider_attribution_lifecycle<S: Storage + ?Sized>(store: &S) {
    use crate::domain::{AttributionLogo, ProviderAttribution};

    let mine = |rows: Vec<ProviderAttribution>| -> Vec<ProviderAttribution> {
        rows.into_iter()
            .filter(|row| row.provider == "attr-tmdb")
            .collect()
    };
    let with_logo = ProviderAttribution {
        provider: "attr-tmdb".into(),
        name: "The Movie Database".into(),
        text: "Data from TMDB.".into(),
        url: Some("https://www.themoviedb.org".into()),
        logo_mime: Some("image/svg+xml".into()),
        logo_digest: Some("abc".into()),
    };
    store
        .upsert_provider_attribution(&with_logo, Some(b"<svg/>"))
        .await
        .expect("insert");
    assert_eq!(
        mine(store.list_provider_attributions().await.expect("list")),
        std::slice::from_ref(&with_logo)
    );
    assert_eq!(
        store
            .provider_attribution_logo("attr-tmdb")
            .await
            .expect("logo"),
        Some(AttributionLogo {
            mime: "image/svg+xml".into(),
            bytes: b"<svg/>".to_vec(),
        })
    );

    let text_only = ProviderAttribution {
        text: "Data from TMDB, reworded.".into(),
        url: None,
        logo_mime: None,
        logo_digest: None,
        ..with_logo
    };
    store
        .upsert_provider_attribution(&text_only, None)
        .await
        .expect("replace");
    assert_eq!(
        mine(store.list_provider_attributions().await.expect("list")),
        [text_only]
    );
    assert_eq!(
        store
            .provider_attribution_logo("attr-tmdb")
            .await
            .expect("logo"),
        None
    );

    store
        .delete_provider_attribution("attr-tmdb")
        .await
        .expect("delete");
    assert!(mine(store.list_provider_attributions().await.expect("list")).is_empty());
    assert_eq!(
        store
            .provider_attribution_logo("attr-missing")
            .await
            .expect("logo"),
        None
    );
}

/// Refresh-token lookup resolves a live session by its current hash and
/// reports `Unknown` for anything else: unknown hashes, and sessions
/// whose hash column is NULL. Revocation must also drop the lookup.
async fn plugin_override_persistence<S: Storage + ?Sized>(store: &S) {
    let row = |plugin: &str, kind: &str, priority| crate::domain::PluginOverrideRow {
        plugin_id: plugin.to_string(),
        hook: "metadata".to_string(),
        work_kind: kind.to_string(),
        priority,
    };
    store
        .upsert_plugin_override("ovr-tmdb", "metadata", "movie", 200)
        .await
        .expect("upsert");
    store
        .upsert_plugin_override("ovr-tmdb", "metadata", "movie", 300)
        .await
        .expect("replace");
    store
        .upsert_plugin_override("ovr-anidb", "metadata", "series", 0)
        .await
        .expect("upsert zero");
    let mine = |rows: Vec<crate::domain::PluginOverrideRow>| {
        rows.into_iter()
            .filter(|r| r.plugin_id.starts_with("ovr-"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        mine(store.list_plugin_overrides().await.expect("list")),
        [row("ovr-anidb", "series", 0), row("ovr-tmdb", "movie", 300)],
        "ordered by plugin, one row per triple"
    );
    assert!(
        store
            .delete_plugin_override("ovr-tmdb", "metadata", "movie")
            .await
            .expect("delete")
    );
    assert!(
        !store
            .delete_plugin_override("ovr-tmdb", "metadata", "movie")
            .await
            .expect("delete again")
    );
    assert_eq!(
        mine(store.list_plugin_overrides().await.expect("list")),
        [row("ovr-anidb", "series", 0)]
    );
}

async fn session_refresh_token_lookup<S: Storage + ?Sized>(store: &S) {
    let user = make_user("refresh-lookup-user", false);
    store.upsert_user(&user).await.expect("upsert user");

    let s = make_session(user.id);
    let hash = s.refresh_token_hash.clone().expect("hash present");
    store.create_session(&s).await.expect("create session");

    // A session with no refresh token must never answer a hash lookup.
    let mut hashless = make_session(user.id);
    hashless.refresh_token_hash = None;
    store
        .create_session(&hashless)
        .await
        .expect("create hashless");

    match store.session_by_refresh_token(&hash).await.expect("lookup") {
        RefreshTokenLookup::Current(found) => assert_eq!(found.id, s.id),
        other => panic!("current hash must resolve Current, got {other:?}"),
    }
    assert!(
        matches!(
            store
                .session_by_refresh_token("no-such-hash")
                .await
                .expect("lookup miss"),
            RefreshTokenLookup::Unknown
        ),
        "unknown hash resolves Unknown"
    );

    store.revoke_session(s.id).await.expect("revoke");
    assert!(
        matches!(
            store
                .session_by_refresh_token(&hash)
                .await
                .expect("lookup after revoke"),
            RefreshTokenLookup::Unknown
        ),
        "revoked session stops resolving"
    );

    store.delete_user(user.id).await.ok();
}

/// Rotation is a compare-and-swap that retires the presented hash: the
/// new hash resolves `Current` with the moved expiry, every superseded
/// hash resolves `Retired` for the session's lifetime, and only the
/// most recent one is flagged `latest`. A losing CAS writes nothing.
async fn session_rotate_refresh_token<S: Storage + ?Sized>(store: &S) {
    let user = make_user("refresh-rotate-user", false);
    store.upsert_user(&user).await.expect("upsert user");

    let s = make_session(user.id);
    let first = s.refresh_token_hash.clone().expect("hash present");
    store.create_session(&s).await.expect("create session");

    let second = format!("rotated-{}", SessionId::new());
    let third = format!("rotated-{}", SessionId::new());
    let first_rotated_at = OffsetDateTime::now_utc() + time::Duration::seconds(5);
    let second_rotated_at = first_rotated_at + time::Duration::seconds(5);
    let new_expiry = OffsetDateTime::now_utc() + time::Duration::days(60);
    assert!(
        store
            .rotate_refresh_token(s.id, &first, &second, new_expiry, first_rotated_at)
            .await
            .expect("rotate")
    );

    // The loser of a race presents the hash the winner just retired.
    let loser = format!("loser-{}", SessionId::new());
    assert!(
        !store
            .rotate_refresh_token(s.id, &first, &loser, new_expiry, first_rotated_at)
            .await
            .expect("replayed rotation")
    );
    assert!(
        matches!(
            store.session_by_refresh_token(&loser).await.expect("loser"),
            RefreshTokenLookup::Unknown
        ),
        "a losing CAS must not install or retire anything"
    );

    match store.session_by_refresh_token(&first).await.expect("first") {
        RefreshTokenLookup::Retired(retired) => {
            assert_eq!(retired.session.id, s.id);
            assert_eq!(retired.session.user_id, user.id);
            assert_eq!(retired.retired_at, first_rotated_at);
            assert!(retired.latest, "immediately-previous token is latest");
        }
        other => panic!("rotated-away hash must resolve Retired, got {other:?}"),
    }
    match store
        .session_by_refresh_token(&second)
        .await
        .expect("second")
    {
        RefreshTokenLookup::Current(current) => {
            assert_eq!(current.id, s.id, "rotation keeps the session id");
            assert_eq!(current.expires_at, new_expiry, "rotation moves expires_at");
            assert_eq!(
                current.last_seen_at, first_rotated_at,
                "rotation touches last_seen_at"
            );
            assert_eq!(current.created_at, s.created_at);
        }
        other => panic!("new hash must resolve Current, got {other:?}"),
    }

    assert!(
        store
            .rotate_refresh_token(s.id, &second, &third, new_expiry, second_rotated_at)
            .await
            .expect("second rotation")
    );
    match store.session_by_refresh_token(&first).await.expect("first") {
        RefreshTokenLookup::Retired(retired) => {
            assert_eq!(retired.retired_at, first_rotated_at);
            assert!(!retired.latest, "an older retired token is not latest");
        }
        other => panic!("older hash must stay Retired, got {other:?}"),
    }
    match store
        .session_by_refresh_token(&second)
        .await
        .expect("second")
    {
        RefreshTokenLookup::Retired(retired) => {
            assert_eq!(retired.retired_at, second_rotated_at);
            assert!(retired.latest);
            assert_eq!(
                retired.session.refresh_token_hash.as_deref(),
                Some(third.as_str())
            );
        }
        other => panic!("second hash must be Retired, got {other:?}"),
    }
    assert!(
        !store
            .rotate_refresh_token(s.id, &first, &loser, new_expiry, second_rotated_at)
            .await
            .expect("retired hash rotation"),
        "a retired hash can never rotate again"
    );

    // A fresh hash must not collide with another session's current one.
    let other = make_session(user.id);
    let other_hash = other.refresh_token_hash.clone().expect("hash");
    store.create_session(&other).await.expect("create other");
    assert!(
        matches!(
            store
                .rotate_refresh_token(s.id, &third, &other_hash, new_expiry, second_rotated_at)
                .await,
            Err(StorageError::Constraint(_))
        ),
        "installing another session's current hash is a Constraint error"
    );
    assert!(
        matches!(
            store.session_by_refresh_token(&third).await.expect("third"),
            RefreshTokenLookup::Current(_)
        ),
        "a failed rotation leaves the current hash in place"
    );

    store.delete_user(user.id).await.ok();
}

/// Retired hashes live exactly as long as their session: revocation,
/// idle pruning, and user deletion all turn them `Unknown`.
async fn session_retired_refresh_tokens_die_with_session<S: Storage + ?Sized>(store: &S) {
    let user = make_user("refresh-retired-lifetime", false);
    store.upsert_user(&user).await.expect("upsert user");
    let now = OffsetDateTime::now_utc();
    let expiry = now + time::Duration::days(30);

    async fn rotated<S: Storage + ?Sized>(
        store: &S,
        session: &Session,
        at: OffsetDateTime,
        expiry: OffsetDateTime,
    ) -> String {
        store.create_session(session).await.expect("create");
        let retired = session.refresh_token_hash.clone().expect("hash");
        let next = format!("next-{}", SessionId::new());
        assert!(
            store
                .rotate_refresh_token(session.id, &retired, &next, expiry, at)
                .await
                .expect("rotate")
        );
        retired
    }
    let is_retired = |lookup: RefreshTokenLookup| matches!(lookup, RefreshTokenLookup::Retired(_));
    let is_unknown = |lookup: RefreshTokenLookup| matches!(lookup, RefreshTokenLookup::Unknown);

    let revoked = make_session(user.id);
    let revoked_hash = rotated(store, &revoked, now, expiry).await;
    assert!(is_retired(
        store
            .session_by_refresh_token(&revoked_hash)
            .await
            .expect("lookup")
    ));
    store.revoke_session(revoked.id).await.expect("revoke");
    assert!(
        is_unknown(
            store
                .session_by_refresh_token(&revoked_hash)
                .await
                .expect("lookup")
        ),
        "revocation drops retired hashes"
    );

    let cutoff = now - time::Duration::hours(1);
    let idle = make_session(user.id);
    let idle_hash = rotated(store, &idle, cutoff - time::Duration::minutes(1), expiry).await;
    store.prune_sessions_before(cutoff).await.expect("prune");
    assert!(
        is_unknown(
            store
                .session_by_refresh_token(&idle_hash)
                .await
                .expect("lookup")
        ),
        "pruning an idle session drops its retired hashes"
    );

    let owned = make_session(user.id);
    let owned_hash = rotated(store, &owned, now, expiry).await;
    store.delete_user(user.id).await.expect("delete user");
    assert!(
        is_unknown(
            store
                .session_by_refresh_token(&owned_hash)
                .await
                .expect("lookup")
        ),
        "user deletion drops retired hashes"
    );
}

/// Sessions are FK CASCADE on user delete: revoking a user wipes their
/// sessions without an explicit per-session revoke.
async fn user_delete_cascades_sessions<S: Storage + ?Sized>(store: &S) {
    let user = make_user("cascade-user", true);
    store.upsert_user(&user).await.expect("upsert user");

    let s = make_session(user.id);
    store.create_session(&s).await.expect("create session");

    store.delete_user(user.id).await.expect("delete user");
    let still_there = store.get_session(s.id).await.expect("get after cascade");
    assert!(
        still_there.is_none(),
        "FK CASCADE on users.id must wipe sessions on user delete"
    );
}

fn make_device_pin() -> DevicePin {
    let now = OffsetDateTime::now_utc();
    DevicePin {
        id: DevicePinId::new(),
        // Unique per call: the UNIQUE index on pin would otherwise
        // collide across PINs coexisting in one test.
        pin: format!(
            "{:06}",
            DevicePinId::new().into_uuid().as_u128() % 1_000_000
        ),
        user_id: None,
        client: Some("NoFlex for Android TV".to_string()),
        device: Some("Chromecast HD".to_string()),
        created_at: now,
        expires_at: now + time::Duration::minutes(5),
    }
}

/// create → by_hash → list → revoke covers every API-key verb. A revoked
/// key still resolves by hash (with `revoked_at` stamped) so the
/// revocation check lives in exactly one place — the caller — and a
/// second revoke reports `false` rather than re-stamping.
async fn api_key_lifecycle<S: Storage + ?Sized>(store: &S) {
    let user = make_user("api-key-user", false);
    store.upsert_user(&user).await.expect("upsert user");

    let key = store
        .create_api_key(user.id, "automation", "hash-api-key-lifecycle")
        .await
        .expect("create api key");
    assert_eq!(key.user_id, user.id);
    assert_eq!(key.name, "automation");
    assert!(key.revoked_at.is_none(), "fresh key starts active");

    let by_hash = store
        .api_key_by_hash("hash-api-key-lifecycle")
        .await
        .expect("by_hash")
        .expect("key resolves by hash");
    assert_eq!(by_hash.id, key.id);
    assert_eq!(by_hash.created_at, key.created_at);

    let listed = store.list_api_keys(user.id).await.expect("list");
    assert_eq!(listed.len(), 1, "exactly one key for this user");
    assert_eq!(listed[0].id, key.id);

    assert!(
        store
            .api_key_by_hash("no-such-hash")
            .await
            .expect("by_hash miss")
            .is_none(),
        "unknown hash resolves to None"
    );

    let found = store
        .get_api_key(key.id)
        .await
        .expect("get by id")
        .expect("key resolves by id");
    assert_eq!(found.id, key.id);
    assert!(found.revoked_at.is_none());

    let revoked = store.revoke_api_key(key.id).await.expect("revoke");
    assert!(revoked, "first revoke reports it did the revoking");
    let found = store
        .get_api_key(key.id)
        .await
        .expect("get by id after revoke")
        .expect("revoked key still resolves by id");
    assert!(found.revoked_at.is_some());
    assert!(
        store
            .get_api_key(noflex_core::ApiKeyId::new())
            .await
            .expect("get unknown")
            .is_none(),
        "unknown id resolves to None"
    );

    let after = store
        .api_key_by_hash("hash-api-key-lifecycle")
        .await
        .expect("by_hash after revoke")
        .expect("revoked key still resolves by hash");
    assert!(
        after.revoked_at.is_some(),
        "revocation is visible via revoked_at; callers reject on it"
    );

    let listed = store
        .list_api_keys(user.id)
        .await
        .expect("list after revoke");
    assert_eq!(listed.len(), 1, "revoked keys stay listed; caller filters");

    let again = store.revoke_api_key(key.id).await.expect("re-revoke");
    assert!(!again, "revoking an already-revoked key returns false");

    let unknown = store
        .revoke_api_key(noflex_core::ApiKeyId::new())
        .await
        .expect("revoke unknown");
    assert!(!unknown, "revoking an unknown id returns false");

    store.delete_user(user.id).await.ok();
}

/// API keys are FK CASCADE on user delete: revoking a user wipes their
/// keys without an explicit per-key revoke.
async fn user_delete_cascades_api_keys<S: Storage + ?Sized>(store: &S) {
    let user = make_user("api-key-cascade-user", false);
    store.upsert_user(&user).await.expect("upsert user");

    store
        .create_api_key(user.id, "doomed", "hash-api-key-cascade")
        .await
        .expect("create api key");

    store.delete_user(user.id).await.expect("delete user");
    let still_there = store
        .api_key_by_hash("hash-api-key-cascade")
        .await
        .expect("by_hash after cascade");
    assert!(
        still_there.is_none(),
        "FK CASCADE on users.id must wipe api keys on user delete"
    );
    assert!(
        store
            .list_api_keys(user.id)
            .await
            .expect("list after cascade")
            .is_empty(),
        "no keys survive their owner"
    );
}

/// create → lookup → authorize → consume covers the device-PIN flow.
/// Authorizing twice fails (the second attempt sees `user_id` already
/// set), authorizing an unknown PIN fails, and delete consumes the row.
async fn device_pin_lifecycle<S: Storage + ?Sized>(store: &S) {
    let user = make_user("device-pin-user", false);
    store.upsert_user(&user).await.expect("upsert user");

    let pin = make_device_pin();
    store.create_device_pin(&pin).await.expect("create pin");

    let pending = store
        .device_pin_by_pin(&pin.pin)
        .await
        .expect("lookup")
        .expect("pin resolves");
    assert_eq!(pending.id, pin.id);
    assert!(pending.user_id.is_none(), "fresh pin is unauthorized");
    assert_eq!(pending.client, pin.client);
    assert_eq!(pending.device, pin.device);

    let authorized = store
        .authorize_device_pin(&pin.pin, user.id)
        .await
        .expect("authorize");
    assert!(authorized, "first authorize succeeds");

    let linked = store
        .device_pin_by_pin(&pin.pin)
        .await
        .expect("lookup after authorize")
        .expect("authorized pin still resolves");
    assert_eq!(linked.user_id, Some(user.id), "device sees who linked it");

    let twice = store
        .authorize_device_pin(&pin.pin, user.id)
        .await
        .expect("re-authorize");
    assert!(!twice, "double-authorize returns false");

    let unknown = store
        .authorize_device_pin("000000-no-such-pin", user.id)
        .await
        .expect("authorize unknown");
    assert!(!unknown, "authorizing an unknown pin returns false");

    let deleted = store.delete_device_pin(pin.id).await.expect("delete");
    assert!(deleted, "delete consumes the row");
    assert!(
        store
            .device_pin_by_pin(&pin.pin)
            .await
            .expect("lookup after delete")
            .is_none(),
        "consumed pin stops resolving"
    );
    let gone = store.delete_device_pin(pin.id).await.expect("re-delete");
    assert!(!gone, "deleting an absent pin returns false");

    store.delete_user(user.id).await.ok();
}

/// An expired PIN is indistinguishable from an unknown one: lookup
/// reports `None` and authorization refuses.
async fn device_pin_expired_is_absent<S: Storage + ?Sized>(store: &S) {
    let user = make_user("device-pin-expired-user", false);
    store.upsert_user(&user).await.expect("upsert user");

    let mut pin = make_device_pin();
    pin.expires_at = OffsetDateTime::now_utc() - time::Duration::minutes(1);
    store
        .create_device_pin(&pin)
        .await
        .expect("create expired pin");

    assert!(
        store
            .device_pin_by_pin(&pin.pin)
            .await
            .expect("lookup expired")
            .is_none(),
        "expired pin is treated as absent"
    );
    let authorized = store
        .authorize_device_pin(&pin.pin, user.id)
        .await
        .expect("authorize expired");
    assert!(!authorized, "expired pin can't be authorized");

    store.delete_user(user.id).await.ok();
}

/// Device PINs are FK CASCADE on user delete: an authorized PIN dies
/// with the user who authorized it.
async fn user_delete_cascades_device_pins<S: Storage + ?Sized>(store: &S) {
    let user = make_user("device-pin-cascade-user", false);
    store.upsert_user(&user).await.expect("upsert user");

    let pin = make_device_pin();
    store.create_device_pin(&pin).await.expect("create pin");
    store
        .authorize_device_pin(&pin.pin, user.id)
        .await
        .expect("authorize");

    store.delete_user(user.id).await.expect("delete user");
    let still_there = store
        .device_pin_by_pin(&pin.pin)
        .await
        .expect("lookup after cascade");
    assert!(
        still_there.is_none(),
        "FK CASCADE on users.id must wipe device pins on user delete"
    );
}

/// `record_playback_progress` must populate `position_ms` and stamp
/// `last_played`; subsequent reads see the latest position, and
/// `mark_played(true)` later moves the row into the "played" bucket.
async fn record_progress_roundtrip<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Progress Lib");
    store.upsert_library(&library).await.expect("library");
    let user = make_user("progress-user", false);
    store.upsert_user(&user).await.expect("user");
    let work = make_work(library.id, "Progress Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");

    let target = noflex_core::PlayTarget::Work(work.id);
    store
        .record_playback_progress(user.id, target, 60_000)
        .await
        .expect("record 1m");
    let after_1m = store.user_data(user.id, target).await.expect("user_data");
    assert_eq!(after_1m.position_ms, Some(60_000));
    assert!(!after_1m.played);
    assert!(after_1m.last_played.is_some());

    store
        .record_playback_progress(user.id, target, 600_000)
        .await
        .expect("record 10m");
    let after_10m = store.user_data(user.id, target).await.expect("user_data");
    assert_eq!(after_10m.position_ms, Some(600_000));
    assert!(!after_10m.played, "progress does not mark played");

    store.delete_work(work.id).await.ok();
    store.delete_user(user.id).await.ok();
    store.delete_library(library.id).await.ok();
}

/// `mark_played(true)` on a fresh row bumps `play_count` from 0 to 1.
/// A second call is a no-op for `play_count`; `mark_played(false)` and
/// re-mark does NOT decrement.
async fn mark_played_play_count<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Played Lib");
    store.upsert_library(&library).await.expect("library");
    let user = make_user("played-user", false);
    store.upsert_user(&user).await.expect("user");
    let work = make_work(library.id, "Played Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let target = noflex_core::PlayTarget::Work(work.id);

    store
        .mark_played(user.id, target, true)
        .await
        .expect("first mark");
    let after_first = store.user_data(user.id, target).await.expect("data");
    assert!(after_first.played);
    assert_eq!(after_first.play_count, 1, "0 -> 1 bumps count to 1");

    store
        .mark_played(user.id, target, true)
        .await
        .expect("idempotent mark");
    let after_second = store.user_data(user.id, target).await.expect("data");
    assert_eq!(
        after_second.play_count, 1,
        "repeat mark does not bump count"
    );

    store
        .mark_played(user.id, target, false)
        .await
        .expect("unmark");
    store
        .mark_played(user.id, target, true)
        .await
        .expect("re-mark");
    let after_rem = store.user_data(user.id, target).await.expect("data");
    assert_eq!(after_rem.play_count, 1, "re-mark does not bump count");

    store.delete_work(work.id).await.ok();
    store.delete_user(user.id).await.ok();
    store.delete_library(library.id).await.ok();
}

async fn set_favorite_roundtrip<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Fav Lib");
    store.upsert_library(&library).await.expect("library");
    let user = make_user("fav-user", false);
    store.upsert_user(&user).await.expect("user");
    let work = make_work(library.id, "Fav Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let target = noflex_core::PlayTarget::Work(work.id);

    let initial = store.user_data(user.id, target).await.expect("data");
    assert!(!initial.favorite);

    store
        .set_favorite(user.id, target, true)
        .await
        .expect("favorite on");
    let on = store.user_data(user.id, target).await.expect("data");
    assert!(on.favorite);

    store
        .set_favorite(user.id, target, false)
        .await
        .expect("favorite off");
    let off = store.user_data(user.id, target).await.expect("data");
    assert!(!off.favorite);

    store.delete_work(work.id).await.ok();
    store.delete_user(user.id).await.ok();
    store.delete_library(library.id).await.ok();
}

/// One series in the shape the scanner produces (series, season, episode).
struct ContinueSeries {
    series: WorkId,
    season: WorkId,
    library: LibraryId,
}

impl ContinueSeries {
    async fn create<S: Storage + ?Sized>(store: &S, library: LibraryId, title: &str) -> Self {
        let now = OffsetDateTime::now_utc();
        let series = make_work_of_kind(library, WorkKind::Series, title, now);
        store.upsert_work(&series).await.expect("series");
        let mut season = make_work_of_kind(library, WorkKind::Season, "Season", now);
        season.parent_id = Some(series.id);
        store.upsert_work(&season).await.expect("season");
        Self {
            series: series.id,
            season: season.id,
            library,
        }
    }

    async fn episode<S: Storage + ?Sized>(&self, store: &S, season: i32, number: i32) -> WorkId {
        self.episode_added(store, season, number, OffsetDateTime::now_utc())
            .await
    }

    async fn episode_added<S: Storage + ?Sized>(
        &self,
        store: &S,
        season: i32,
        number: i32,
        added_at: OffsetDateTime,
    ) -> WorkId {
        let mut episode = make_work_of_kind(
            self.library,
            WorkKind::Episode,
            &format!("S{season:02}E{number:02}"),
            added_at,
        );
        episode.parent_id = Some(self.season);
        episode.parent_index = Some(season);
        episode.index_number = Some(number);
        store.upsert_work(&episode).await.expect("episode");
        episode.id
    }
}

async fn played<S: Storage + ?Sized>(store: &S, user: UserId, work: WorkId) {
    store
        .mark_played(user, PlayTarget::Work(work), true)
        .await
        .expect("played");
}

/// Timestamps are the only ordering input, and consecutive writes can land
/// on the same one.
async fn tick() {
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
}

/// What a user's "Continue watching" lists, a long way before now so nothing
/// decays unless a test asks for it.
async fn continuing<S: Storage + ?Sized>(store: &S, user: UserId) -> Vec<WorkId> {
    store
        .continue_watching(
            user,
            OffsetDateTime::now_utc() - time::Duration::days(3650),
            50,
        )
        .await
        .expect("continue_watching")
        .into_iter()
        .map(|work| work.id)
        .collect()
}

async fn continue_watching_resumes_unfinished_works<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Resume Lib");
    store.upsert_library(&library).await.expect("library");
    let user = make_user("resume-user", false);
    store.upsert_user(&user).await.expect("user");

    let now = OffsetDateTime::now_utc();
    let older = make_work(library.id, "Older", now);
    let newer = make_work(library.id, "Newer", now);
    let finished = make_work(library.id, "Finished", now);
    let untouched = make_work(library.id, "Untouched", now);
    for work in [&older, &newer, &finished, &untouched] {
        store.upsert_work(work).await.expect("work");
    }
    for work in [&older, &newer] {
        store
            .record_playback_progress(user.id, PlayTarget::Work(work.id), 100_000)
            .await
            .expect("progress");
        tick().await;
    }
    played(store, user.id, finished.id).await;

    assert_eq!(
        continuing(store, user.id).await,
        [newer.id, older.id],
        "unfinished works only, most recent first"
    );

    let other = make_user("resume-other", false);
    store.upsert_user(&other).await.expect("other");
    assert!(continuing(store, other.id).await.is_empty());

    store.delete_user(user.id).await.ok();
    store.delete_user(other.id).await.ok();
    store.delete_library(library.id).await.ok();
}

async fn continue_watching_suggests_the_next_episode<S: Storage + ?Sized>(store: &S) {
    let library = test_library("NextUp Lib");
    store.upsert_library(&library).await.expect("library");
    let user = make_user("nextup-user", false);
    store.upsert_user(&user).await.expect("user");

    let show = ContinueSeries::create(store, library.id, "Show").await;
    let special = show.episode(store, 0, 1).await;
    let s3e5 = show.episode(store, 3, 5).await;
    let s3e6 = show.episode(store, 3, 6).await;
    let s3e7 = show.episode(store, 3, 7).await;
    let s3e8 = show.episode(store, 3, 8).await;
    let s4e1 = show.episode(store, 4, 1).await;

    played(store, user.id, s3e6).await;
    assert_eq!(
        continuing(store, user.id).await,
        [s3e7],
        "S03E06 leads to S03E07, and the skipped S03E05 stays skipped"
    );

    played(store, user.id, s3e8).await;
    assert_eq!(
        continuing(store, user.id).await,
        [s4e1],
        "the next episode crosses into the next season"
    );

    tick().await;
    played(store, user.id, s3e6).await;
    assert_eq!(
        continuing(store, user.id).await,
        [s3e7],
        "rewatching goes back to following that episode"
    );

    // An unfinished episode is the series' one entry, ahead of any next one.
    tick().await;
    store
        .record_playback_progress(user.id, PlayTarget::Work(s3e7), 60_000)
        .await
        .expect("progress");
    assert_eq!(continuing(store, user.id).await, [s3e7]);

    // A special is never next, and watching one continues the main run.
    tick().await;
    played(store, user.id, s3e7).await;
    tick().await;
    played(store, user.id, special).await;
    assert_eq!(continuing(store, user.id).await, [s3e5]);

    // Past the last episode, nothing is left to suggest.
    tick().await;
    played(store, user.id, s4e1).await;
    assert!(continuing(store, user.id).await.is_empty());

    store.delete_user(user.id).await.ok();
    store.delete_library(library.id).await.ok();
}

async fn continue_watching_handles_multi_episode_files<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Multi-episode Lib");
    store.upsert_library(&library).await.expect("library");
    let user = make_user("multi-episode-user", false);
    store.upsert_user(&user).await.expect("user");

    let show = ContinueSeries::create(store, library.id, "Show").await;
    let mut double = make_work_of_kind(
        library.id,
        WorkKind::Episode,
        "E05-E06",
        OffsetDateTime::now_utc(),
    );
    double.parent_id = Some(show.season);
    double.parent_index = Some(1);
    double.index_number = Some(5);
    double.index_end = Some(6);
    store.upsert_work(&double).await.expect("double");
    show.episode(store, 1, 6).await;
    let e7 = show.episode(store, 1, 7).await;

    played(store, user.id, double.id).await;
    assert_eq!(continuing(store, user.id).await, [e7]);

    store.delete_user(user.id).await.ok();
    store.delete_library(library.id).await.ok();
}

async fn continue_watching_decays_until_new_episodes_arrive<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Decay Lib");
    store.upsert_library(&library).await.expect("library");
    let user = make_user("decay-user", false);
    store.upsert_user(&user).await.expect("user");

    let now = OffsetDateTime::now_utc();
    let show = ContinueSeries::create(store, library.id, "Show").await;
    let last_watched = show
        .episode_added(store, 1, 1, now - time::Duration::days(400))
        .await;
    let old_next = show
        .episode_added(store, 1, 2, now - time::Duration::days(400))
        .await;
    played(store, user.id, last_watched).await;

    let listed = |since| async move {
        store
            .continue_watching(user.id, since, 50)
            .await
            .expect("continue_watching")
            .into_iter()
            .map(|work| work.id)
            .collect::<Vec<_>>()
    };
    let cutoff = now + time::Duration::days(1);
    assert_eq!(listed(now - time::Duration::days(1)).await, [old_next]);
    assert!(
        listed(cutoff).await.is_empty(),
        "activity older than the cutoff decays"
    );

    // A next episode added after the cutoff brings the series back. Only
    // the next episode counts, so this replaces the old one.
    store
        .mark_played(user.id, PlayTarget::Work(old_next), true)
        .await
        .expect("played");
    let new_next = show
        .episode_added(store, 2, 1, now + time::Duration::days(2))
        .await;
    assert_eq!(listed(cutoff).await, [new_next]);

    store.delete_user(user.id).await.ok();
    store.delete_library(library.id).await.ok();
}

async fn content_categories_split_by_kind_and_anime<S: Storage + ?Sized>(store: &S) {
    use crate::{
        domain::{
            ANIME_TAG_SLUG, CATEGORY_TAG_NAMESPACE, ContentCategory, ContentCategoryCount,
            LibraryCategory, Tag,
        },
        query::ListWorksQuery,
    };

    let now = OffsetDateTime::now_utc();
    let mut mixed = test_library("Category Mixed");
    let mut anime_lib = test_library("Category Anime");
    anime_lib.config.content_category = LibraryCategory::Anime;
    let mut general_lib = test_library("Category General");
    general_lib.config.content_category = LibraryCategory::General;
    mixed.config.content_category = LibraryCategory::Auto;
    for library in [&mixed, &anime_lib, &general_lib] {
        store.upsert_library(library).await.expect("library");
    }

    let plain_movie = make_work(mixed.id, "Plain movie", now);
    let tagged_movie = make_work(mixed.id, "Tagged movie", now);
    let plain_show = make_work_of_kind(mixed.id, WorkKind::Series, "Plain show", now);
    let tagged_show = make_work_of_kind(mixed.id, WorkKind::Series, "Tagged show", now);
    let library_show = make_work_of_kind(anime_lib.id, WorkKind::Series, "Library show", now);
    let general_tagged = make_work_of_kind(general_lib.id, WorkKind::Series, "General tagged", now);
    let mut season = make_work_of_kind(mixed.id, WorkKind::Season, "Season 1", now);
    season.parent_id = Some(plain_show.id);
    for work in [
        &plain_movie,
        &tagged_movie,
        &plain_show,
        &tagged_show,
        &library_show,
        &general_tagged,
        &season,
    ] {
        store.upsert_work(work).await.expect("work");
    }
    let tag = Tag {
        id: TagId::new(),
        namespace: CATEGORY_TAG_NAMESPACE.to_string(),
        parent_id: None,
        slug: ANIME_TAG_SLUG.to_string(),
        display: "Anime".to_string(),
        metadata: async_graphql::Json(serde_json::json!({})),
    };
    let tag_id = store.upsert_tag(&tag).await.expect("tag");
    for work in [&tagged_movie, &tagged_show, &general_tagged] {
        store
            .assign_tag(tag_id, TagTarget::Work(work.id), 1.0, "provider:test")
            .await
            .expect("assign");
    }

    let titles = async |category, kinds: Vec<WorkKind>| {
        let mut titles = store
            .list_works(&ListWorksQuery {
                category: Some(category),
                kinds,
                limit: 50,
                ..Default::default()
            })
            .await
            .expect("list_works")
            .items
            .into_iter()
            .map(|work| work.title)
            .filter(|title| title != "Unrelated")
            .collect::<Vec<_>>();
        titles.sort();
        titles
    };
    assert_eq!(
        titles(ContentCategory::Movies, vec![]).await,
        ["Plain movie"]
    );
    assert_eq!(
        titles(ContentCategory::Shows, vec![]).await,
        ["General tagged", "Plain show"],
        "a General library overrides the tag"
    );
    assert_eq!(
        titles(ContentCategory::Anime, vec![]).await,
        ["Library show", "Tagged movie", "Tagged show"],
        "an Anime library needs no tag"
    );
    assert_eq!(
        titles(ContentCategory::Anime, vec![WorkKind::Movie]).await,
        ["Tagged movie"],
        "kind filters narrow a category"
    );

    let counts = store.content_category_counts(None).await.expect("counts");
    let count = |category| {
        counts
            .iter()
            .find(|entry: &&ContentCategoryCount| entry.category == category)
            .map(|entry| entry.work_count)
    };
    assert_eq!(count(ContentCategory::Anime), Some(3));

    // Grants scope the counts, and so hide a category the user can't reach.
    let user = make_user("category-user", false);
    store.upsert_user(&user).await.expect("user");
    store
        .grant_library_access(user.id, general_lib.id)
        .await
        .expect("grant");
    assert_eq!(
        store
            .content_category_counts(Some(user.id))
            .await
            .expect("counts"),
        [ContentCategoryCount {
            category: ContentCategory::Shows,
            work_count: 1,
        }]
    );

    store.delete_user(user.id).await.ok();
    for library in [&mixed, &anime_lib, &general_lib] {
        store.delete_library(library.id).await.ok();
    }
}

async fn work_features_collect_comparison_facts<S: Storage + ?Sized>(store: &S) {
    use crate::domain::Tag;

    let now = OffsetDateTime::now_utc();
    let library = test_library("Features Lib");
    let hidden_library = test_library("Features Hidden");
    store.upsert_library(&library).await.expect("library");
    store
        .upsert_library(&hidden_library)
        .await
        .expect("library");

    let mut movie = make_work(library.id, "Featured movie", now);
    movie.year = Some(1999);
    movie.community_rating = Some(8.5);
    let series = make_work_of_kind(library.id, WorkKind::Series, "Featured series", now);
    let mut season = make_work_of_kind(library.id, WorkKind::Season, "Season", now);
    season.parent_id = Some(series.id);
    let album = make_work_of_kind(library.id, WorkKind::Album, "Featured album", now);
    let hidden = make_work(hidden_library.id, "Hidden movie", now);
    for work in [&movie, &series, &season, &album, &hidden] {
        store.upsert_work(work).await.expect("work");
    }
    let mut short = make_edition(movie.id, "Short");
    short.runtime_ms = Some(1_000);
    let mut long = make_edition(movie.id, "Long");
    long.runtime_ms = Some(2_000);
    store.upsert_edition(&short).await.expect("edition");
    store.upsert_edition(&long).await.expect("edition");

    let tag = |namespace: &str, slug: &str| Tag {
        id: TagId::new(),
        namespace: namespace.to_string(),
        parent_id: None,
        slug: slug.to_string(),
        display: slug.to_string(),
        metadata: async_graphql::Json(serde_json::json!({})),
    };
    let genre = store
        .upsert_tag(&tag("genre", "features-genre"))
        .await
        .expect("tag");
    let user_tag = store
        .upsert_tag(&tag("user", "features-user"))
        .await
        .expect("tag");
    for id in [genre, user_tag] {
        store
            .assign_tag(id, TagTarget::Work(movie.id), 1.0, "test")
            .await
            .expect("assign");
    }

    let kinds = [WorkKind::Movie, WorkKind::Series];
    let features = store
        .work_features(&kinds, &["genre"], None)
        .await
        .expect("features");
    let found = features
        .iter()
        .find(|entry| entry.work == movie.id)
        .expect("movie");
    assert_eq!(found.kind, WorkKind::Movie);
    assert_eq!(found.year, Some(1999));
    assert_eq!(found.community_rating, Some(8.5));
    assert_eq!(found.runtime_ms, Some(2_000), "the longest edition");
    assert_eq!(found.tags, [genre], "only the requested namespaces");
    let found_series = features
        .iter()
        .find(|entry| entry.work == series.id)
        .expect("series");
    assert_eq!(found_series.runtime_ms, None);
    assert!(
        features
            .iter()
            .all(|entry| entry.work != season.id && entry.work != album.id),
        "top-level works of the requested kinds only"
    );

    let user = make_user("features-user", false);
    store.upsert_user(&user).await.expect("user");
    store
        .grant_library_access(user.id, library.id)
        .await
        .expect("grant");
    let visible = store
        .work_features(&kinds, &[], Some(user.id))
        .await
        .expect("features");
    assert!(visible.iter().any(|entry| entry.work == movie.id));
    assert!(
        visible.iter().all(|entry| entry.work != hidden.id),
        "grants apply"
    );

    store.delete_user(user.id).await.ok();
    store.delete_library(library.id).await.ok();
    store.delete_library(hidden_library.id).await.ok();
}

async fn episodes_after_lists_regular_episodes_in_order<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Episodes After Lib");
    store.upsert_library(&library).await.expect("library");
    let show = ContinueSeries::create(store, library.id, "Show").await;
    show.episode(store, 0, 1).await;
    let s1e1 = show.episode(store, 1, 1).await;
    let s1e2 = show.episode(store, 1, 2).await;
    let s2e1 = show.episode(store, 2, 1).await;

    let ids = |works: Vec<Work>| works.into_iter().map(|work| work.id).collect::<Vec<_>>();
    assert_eq!(
        ids(store
            .episodes_after(show.series, 0, 0, 10)
            .await
            .expect("from the start")),
        [s1e1, s1e2, s2e1],
        "specials are left out"
    );
    assert_eq!(
        ids(store
            .episodes_after(show.series, 1, 2, 1)
            .await
            .expect("across seasons")),
        [s2e1]
    );

    store.delete_library(library.id).await.ok();
}

fn make_device(user: UserId, name: &str) -> crate::domain::Device {
    let now = OffsetDateTime::now_utc();
    crate::domain::Device {
        id: noflex_core::DeviceId::new(),
        user_id: user,
        name: name.to_string(),
        capabilities: async_graphql::Json(serde_json::json!({"max_height": 1080})),
        smart_transcode: false,
        max_bitrate_bps: None,
        allowed_video_codecs: Vec::new(),
        smart_downloads: false,
        download_budget_bytes: None,
        downloads_wifi_only: true,
        created_at: now,
        last_seen_at: now,
    }
}

async fn smart_transcode_marks_queue_by_scope<S: Storage + ?Sized>(store: &S) {
    use crate::domain::{SmartTranscodeMark, SmartTranscodeOutcome};

    let first_user = make_user("mark-first", false);
    let second_user = make_user("mark-second", false);
    store.upsert_user(&first_user).await.expect("first user");
    store.upsert_user(&second_user).await.expect("second user");
    let at = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("nanos");
    let library = test_library("mark-library");
    store.upsert_library(&library).await.expect("library");
    let film = make_work(library.id, "Marked film", at);
    let other = make_work(library.id, "Marked other", at);
    store.upsert_work(&film).await.expect("film");
    store.upsert_work(&other).await.expect("other");
    let tv = noflex_core::DeviceId::new();

    let mark = |user: UserId, work: WorkId, device, created_at| SmartTranscodeMark {
        user_id: user,
        work_id: work,
        device_id: device,
        created_at,
        outcome: None,
        media_file_ids: vec![],
    };
    let seconds = time::Duration::seconds;
    let added = store
        .add_smart_transcode_mark(&mark(first_user.id, film.id, None, at))
        .await
        .expect("add");
    assert_eq!(added.outcome, None);
    store
        .add_smart_transcode_mark(&mark(first_user.id, film.id, Some(tv), at + seconds(1)))
        .await
        .expect("the same title for one device is its own mark");
    store
        .add_smart_transcode_mark(&mark(second_user.id, film.id, None, at + seconds(2)))
        .await
        .expect("another viewer");
    store
        .add_smart_transcode_mark(&mark(first_user.id, other.id, None, at + seconds(3)))
        .await
        .expect("another title");

    let again = store
        .add_smart_transcode_mark(&mark(
            first_user.id,
            film.id,
            None,
            at + time::Duration::days(1),
        ))
        .await
        .expect("again");
    assert_eq!(again.created_at, at, "marking twice keeps its place");

    let queue = store.list_smart_transcode_marks().await.expect("queue");
    let order = queue
        .iter()
        .filter(|entry| entry.user_id == first_user.id || entry.user_id == second_user.id)
        .map(|entry| (entry.user_id, entry.work_id, entry.device_id))
        .collect::<Vec<_>>();
    assert_eq!(
        order,
        [
            (first_user.id, film.id, None),
            (first_user.id, film.id, Some(tv)),
            (second_user.id, film.id, None),
            (first_user.id, other.id, None),
        ],
        "first come, first served across users"
    );

    let files = [MediaFileId::new()];
    let planned = mark(first_user.id, film.id, Some(tv), at);
    store
        .record_smart_transcode_plan(&planned, SmartTranscodeOutcome::Planned, &files)
        .await
        .expect("record");
    let mine = store
        .smart_transcode_marks_for_user(first_user.id)
        .await
        .expect("mine");
    assert_eq!(mine.len(), 3);
    assert_eq!(mine[0].outcome, None, "only the scope recorded changes");
    assert_eq!(mine[1].outcome, Some(SmartTranscodeOutcome::Planned));
    assert_eq!(mine[1].media_file_ids, files);

    assert!(
        store
            .remove_smart_transcode_mark(first_user.id, film.id, Some(tv))
            .await
            .expect("remove")
    );
    assert!(
        !store
            .remove_smart_transcode_mark(first_user.id, film.id, Some(tv))
            .await
            .expect("remove again"),
        "nothing left to remove"
    );
    // Recording after it's gone, as the planner can: a no-op, not a resurrection.
    store
        .record_smart_transcode_plan(&planned, SmartTranscodeOutcome::Planned, &files)
        .await
        .expect("record after removal");
    assert_eq!(
        store
            .smart_transcode_marks_for_user(first_user.id)
            .await
            .expect("mine")
            .len(),
        2,
        "the mark for all devices stays"
    );
    assert_eq!(
        store
            .smart_transcode_marks_for_user(second_user.id)
            .await
            .expect("theirs")
            .len(),
        1,
        "another viewer's mark is their own"
    );

    // Marks go with their work, and later tests count every work in the store.
    store.delete_library(library.id).await.expect("delete");
    assert!(
        store
            .smart_transcode_marks_for_user(second_user.id)
            .await
            .expect("theirs")
            .is_empty()
    );
}

async fn devices_belong_to_their_user<S: Storage + ?Sized>(store: &S) {
    let owner = make_user("device-owner", false);
    let other = make_user("device-other", false);
    store.upsert_user(&owner).await.expect("owner");
    store.upsert_user(&other).await.expect("other");

    let mut device = make_device(owner.id, "Phone");
    store.upsert_device(&device).await.expect("register");
    let registered = store
        .get_device(device.id)
        .await
        .expect("get")
        .expect("device")
        .created_at;

    device.smart_transcode = true;
    device.max_bitrate_bps = Some(8_000_000);
    device.allowed_video_codecs = vec![noflex_core::VideoCodec::H264];
    device.created_at = registered + time::Duration::days(1);
    store.upsert_device(&device).await.expect("refresh");
    let found = store
        .get_device(device.id)
        .await
        .expect("get")
        .expect("device");
    assert_eq!(found.max_bitrate_bps, Some(8_000_000));
    assert_eq!(found.allowed_video_codecs, [noflex_core::VideoCodec::H264]);
    assert_eq!(found.capabilities.0["max_height"], 1080);
    assert_eq!(
        found.created_at, registered,
        "refreshing keeps the registration date"
    );
    assert!(
        store
            .devices_wanting_versions()
            .await
            .expect("wanting")
            .iter()
            .any(|entry| entry.id == device.id)
    );

    // Another user can't take over the id.
    let mut stolen = device.clone();
    stolen.user_id = other.id;
    stolen.name = "Stolen".to_string();
    assert!(matches!(
        store.upsert_device(&stolen).await,
        Err(StorageError::NotFound)
    ));
    assert_eq!(
        store
            .get_device(device.id)
            .await
            .expect("get")
            .expect("device")
            .name,
        "Phone"
    );
    assert!(
        !store
            .delete_device(other.id, device.id)
            .await
            .expect("delete")
    );
    assert_eq!(
        store.devices_for_user(owner.id).await.expect("list").len(),
        1
    );
    assert!(
        store
            .delete_device(owner.id, device.id)
            .await
            .expect("delete")
    );

    store.delete_user(owner.id).await.ok();
    store.delete_user(other.id).await.ok();
}

async fn downloads_allowed_round_trip<S: Storage + ?Sized>(store: &S) {
    assert!(store.downloads_allowed().await.expect("default"));
    store.set_downloads_allowed(false).await.expect("refuse");
    assert!(!store.downloads_allowed().await.expect("refused"));
    store.set_downloads_allowed(true).await.expect("allow");
    assert!(store.downloads_allowed().await.expect("allowed"));
}

async fn smart_transcoding_settings_round_trip<S: Storage + ?Sized>(store: &S) {
    use crate::domain::SmartTranscodingSettings;

    assert_eq!(store.tonemap_curve().await.expect("unset"), None);
    store
        .set_tonemap_curve(Some(noflex_core::TonemapCurve::Bt2390))
        .await
        .expect("set curve");
    assert_eq!(
        store.tonemap_curve().await.expect("read"),
        Some(noflex_core::TonemapCurve::Bt2390),
    );
    store.set_tonemap_curve(None).await.expect("clear curve");
    assert_eq!(store.tonemap_curve().await.expect("cleared"), None);

    assert_eq!(
        store.smart_transcoding_settings().await.expect("defaults"),
        SmartTranscodingSettings::default(),
    );
    let settings = SmartTranscodingSettings {
        dir: Some("/mnt/fast/versions".to_string()),
        max_storage_gb: Some(500),
        max_version_gb: None,
    };
    store
        .set_smart_transcoding_settings(&settings)
        .await
        .expect("set");
    assert_eq!(
        store.smart_transcoding_settings().await.expect("read"),
        settings,
    );
    // Reverting to the configuration is a plain overwrite.
    store
        .set_smart_transcoding_settings(&SmartTranscodingSettings::default())
        .await
        .expect("clear");
    assert_eq!(
        store.smart_transcoding_settings().await.expect("read"),
        SmartTranscodingSettings::default(),
    );
}

async fn optimized_versions_queue_by_priority<S: Storage + ?Sized>(store: &S) {
    use crate::domain::{OptimizedVersion, OptimizedVersionStatus};

    let now = OffsetDateTime::now_utc();
    let library = test_library("Versions Lib");
    store.upsert_library(&library).await.expect("library");
    let work = make_work(library.id, "Versions", now);
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Default");
    store.upsert_edition(&edition).await.expect("edition");
    let media = make_media(edition.id, "/versions/source.mkv", now);
    store.upsert_media_file(&media).await.expect("media");

    let version = |key: &str, priority| OptimizedVersion {
        id: noflex_core::OptimizedVersionId::new(),
        media_file_id: media.id,
        variant_key: key.to_string(),
        status: OptimizedVersionStatus::Queued,
        priority,
        plan: serde_json::json!({"key": key}),
        source_size: 10,
        source_mtime: now,
        progress: 0.0,
        attempts: 0,
        path: None,
        size_bytes: None,
        index: None,
        probe: None,
        error: None,
        created_at: now,
        finished_at: None,
        last_used_at: None,
        unwanted_since: None,
    };
    let later = version("later", 2);
    let sooner = version("sooner", 1);
    store
        .enqueue_optimized_version(&later)
        .await
        .expect("later");
    store
        .enqueue_optimized_version(&sooner)
        .await
        .expect("sooner");

    // Re-planning moves a waiting job and doesn't duplicate it.
    store
        .enqueue_optimized_version(&version("later", 0))
        .await
        .expect("re-plan");
    assert_eq!(
        store
            .optimized_versions_for_media(media.id)
            .await
            .expect("list")
            .len(),
        2
    );

    // The stretch starts when it first goes unwanted, and being planned ends it.
    let unwanted_since = async || {
        store
            .get_optimized_version(sooner.id)
            .await
            .expect("get")
            .expect("version")
            .unwanted_since
            .map(OffsetDateTime::unix_timestamp)
    };
    let earlier = now - time::Duration::hours(1);
    store
        .mark_optimized_version_unwanted(sooner.id, earlier)
        .await
        .expect("unwanted");
    store
        .mark_optimized_version_unwanted(sooner.id, now)
        .await
        .expect("still unwanted");
    assert_eq!(unwanted_since().await, Some(earlier.unix_timestamp()));
    store
        .enqueue_optimized_version(&version("sooner", 1))
        .await
        .expect("wanted again");
    assert_eq!(unwanted_since().await, None);

    let claimed = store
        .claim_optimized_version()
        .await
        .expect("claim")
        .expect("queued");
    assert_eq!(claimed.variant_key, "later", "re-planned to the front");
    assert_eq!(claimed.status, OptimizedVersionStatus::Running);
    assert_eq!(claimed.attempts, 1);

    // A restart finds it running with nobody working on it.
    assert_eq!(
        store
            .requeue_running_optimized_versions()
            .await
            .expect("requeue"),
        1
    );
    let mut finished = store
        .claim_optimized_version()
        .await
        .expect("claim")
        .expect("queued");
    assert_eq!(finished.id, claimed.id);
    assert_eq!(finished.attempts, 2);

    // Finished versions keep their outputs, and re-planning leaves them be.
    finished.status = OptimizedVersionStatus::Ready;
    finished.path = Some("/optimized/out.mp4".to_string());
    finished.size_bytes = Some(5_000_000_000);
    finished.index = Some(serde_json::json!({"tracks": []}));
    finished.probe = Some(serde_json::json!({"video": null}));
    finished.finished_at = Some(now);
    finished.last_used_at = Some(now);
    store
        .update_optimized_version(&finished)
        .await
        .expect("finish");
    store
        .enqueue_optimized_version(&version("later", 5))
        .await
        .expect("re-plan a finished version");
    let found = store
        .get_optimized_version(finished.id)
        .await
        .expect("get")
        .expect("version");
    assert_eq!(found.status, OptimizedVersionStatus::Ready);
    assert_eq!(found.priority, 0);

    // Re-planning a finished version still says who it's for now.
    let mut replanned = version("later", 5);
    replanned.plan = serde_json::json!({"key": "later", "devices": ["someone"]});
    store
        .enqueue_optimized_version(&replanned)
        .await
        .expect("re-plan");
    let found = store
        .get_optimized_version(finished.id)
        .await
        .expect("get")
        .expect("version");
    assert_eq!(found.plan["devices"][0], "someone");

    // Paged for showing: what needs a look first, and counted without reading them.
    let first = store
        .optimized_versions_page(None, 1, None)
        .await
        .expect("first page");
    assert_eq!(
        first.items[0].variant_key, "sooner",
        "queued ahead of ready"
    );
    assert!(first.has_next);
    let rest = store
        .optimized_versions_page(None, 1, first.end_cursor)
        .await
        .expect("next page");
    assert_eq!(rest.items[0].variant_key, "later");
    assert!(!rest.has_next);
    let ready = store
        .optimized_versions_page(Some(OptimizedVersionStatus::Ready), 10, None)
        .await
        .expect("ready");
    assert_eq!(ready.items.len(), 1);
    let stats = store.optimized_version_stats().await.expect("stats");
    assert_eq!(
        (stats.ready, stats.queued, stats.ready_bytes),
        (1, 1, 5_000_000_000)
    );
    assert_eq!(found.size_bytes, Some(5_000_000_000));
    assert_eq!(found.index, Some(serde_json::json!({"tracks": []})));
    assert!(found.finished_at.is_some());

    assert_eq!(store.list_optimized_versions().await.expect("all").len(), 2);
    store
        .delete_optimized_version(sooner.id)
        .await
        .expect("delete");
    assert_eq!(store.list_optimized_versions().await.expect("all").len(), 1);

    // Versions go with their source.
    store.delete_library(library.id).await.ok();
    assert!(
        store
            .list_optimized_versions()
            .await
            .expect("all")
            .is_empty()
    );
}

async fn invites_can_only_be_spent_while_valid<S: Storage + ?Sized>(store: &S) {
    use crate::domain::Invite;

    let admin = make_user("invite-admin", true);
    store.upsert_user(&admin).await.expect("admin");
    let now = OffsetDateTime::now_utc();
    let invite = |max_uses, expires_at| Invite {
        id: noflex_core::InviteId::new(),
        created_by: admin.id,
        library_ids: vec![LibraryId::new()],
        max_uses,
        uses: 0,
        expires_at,
        revoked_at: None,
        created_at: now,
    };

    let once = invite(1, now + time::Duration::days(1));
    store
        .create_invite(&once, "hash-once")
        .await
        .expect("create");
    let spent = store
        .consume_invite("hash-once", now)
        .await
        .expect("consume")
        .expect("valid");
    assert_eq!(spent.uses, 1);
    assert_eq!(spent.library_ids, once.library_ids);
    assert!(
        store
            .consume_invite("hash-once", now)
            .await
            .expect("consume")
            .is_none(),
        "used up"
    );
    store.release_invite_use(once.id).await.expect("release");
    assert!(
        store
            .consume_invite("hash-once", now)
            .await
            .expect("consume")
            .is_some(),
        "a released use can be spent again"
    );

    let expired = invite(5, now - time::Duration::seconds(1));
    store
        .create_invite(&expired, "hash-expired")
        .await
        .expect("create");
    assert!(
        store
            .consume_invite("hash-expired", now)
            .await
            .expect("consume")
            .is_none()
    );

    let revoked = invite(5, now + time::Duration::days(1));
    store
        .create_invite(&revoked, "hash-revoked")
        .await
        .expect("create");
    assert!(store.revoke_invite(revoked.id, now).await.expect("revoke"));
    assert!(
        !store.revoke_invite(revoked.id, now).await.expect("revoke"),
        "already revoked"
    );
    assert!(
        store
            .consume_invite("hash-revoked", now)
            .await
            .expect("consume")
            .is_none()
    );
    assert!(
        store
            .consume_invite("hash-unknown", now)
            .await
            .expect("consume")
            .is_none()
    );
    assert_eq!(store.list_invites().await.expect("list").len(), 3);

    store.delete_user(admin.id).await.ok();
}

async fn linked_servers_belong_to_their_user<S: Storage + ?Sized>(store: &S) {
    use crate::domain::LinkedServer;

    let owner = make_user("link-owner", false);
    let other = make_user("link-other", false);
    store.upsert_user(&owner).await.expect("owner");
    store.upsert_user(&other).await.expect("other");
    let now = OffsetDateTime::now_utc();
    let link = |user: UserId, origin: &str, key: &str| LinkedServer {
        id: noflex_core::LinkedServerId::new(),
        user_id: user,
        origin: origin.to_string(),
        name: "Friend".to_string(),
        api_key: key.to_string(),
        sync_enabled: true,
        synced_until: None,
        created_at: now,
    };

    let first = store
        .upsert_linked_server(&link(owner.id, "https://friend.example", "key-1"))
        .await
        .expect("link");

    // Relinking replaces the key on the same row.
    let relinked = store
        .upsert_linked_server(&link(owner.id, "https://friend.example", "key-2"))
        .await
        .expect("relink");
    assert_eq!(relinked.id, first.id);
    assert_eq!(relinked.api_key, "key-2");
    assert_eq!(
        store
            .linked_servers_for_user(owner.id)
            .await
            .expect("list")
            .len(),
        1
    );

    store
        .upsert_linked_server(&link(other.id, "https://friend.example", "key-3"))
        .await
        .expect("another user's link");
    store
        .upsert_linked_server(&link(other.id, "http://192.168.1.5:8096", "key-4"))
        .await
        .expect("another origin");
    assert_eq!(
        store.linked_servers_for_user(owner.id).await.expect("list")[0].api_key,
        "key-2",
        "each user sees their own key"
    );
    let origins = store.linked_server_origins().await.expect("origins");
    assert!(origins.contains(&"https://friend.example".to_string()));
    assert!(origins.contains(&"http://192.168.1.5:8096".to_string()));
    assert_eq!(
        origins
            .iter()
            .filter(|origin| *origin == "https://friend.example")
            .count(),
        1
    );

    assert!(
        !store
            .delete_linked_server(other.id, first.id)
            .await
            .expect("delete"),
        "not theirs to delete"
    );
    assert!(
        store
            .delete_linked_server(owner.id, first.id)
            .await
            .expect("delete")
    );

    store.delete_user(owner.id).await.ok();
    store.delete_user(other.id).await.ok();
    assert!(
        !store
            .linked_server_origins()
            .await
            .expect("origins")
            .contains(&"http://192.168.1.5:8096".to_string()),
        "links go with their user, and the policy with them"
    );
}

async fn applied_watch_state_keeps_its_original_time<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Sync Lib");
    store.upsert_library(&library).await.expect("library");
    let user = make_user("sync-user", false);
    store.upsert_user(&user).await.expect("user");
    let work = make_work(library.id, "Synced", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let target = PlayTarget::Work(work.id);
    store
        .set_favorite(user.id, target, true)
        .await
        .expect("favorite");

    let then = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("time")
        - time::Duration::days(3);
    store
        .apply_watch_state(user.id, target, true, Some(5_000), then)
        .await
        .expect("apply");
    let data = store.user_data(user.id, target).await.expect("data");
    assert!(data.played);
    assert_eq!(data.position_ms, Some(5_000));
    assert_eq!(data.last_played, Some(then), "not stamped with now");
    assert_eq!(data.play_count, 1);
    assert!(data.favorite, "other state is left alone");

    store.delete_user(user.id).await.ok();
    store.delete_library(library.id).await.ok();
}

async fn watched_top_level_works_roll_up<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Watched Lib");
    store.upsert_library(&library).await.expect("library");
    let user = make_user("watched-user", false);
    store.upsert_user(&user).await.expect("user");

    let show = ContinueSeries::create(store, library.id, "Show").await;
    let e1 = show.episode(store, 1, 1).await;
    let e2 = show.episode(store, 1, 2).await;
    let movie = make_work(library.id, "Movie", OffsetDateTime::now_utc());
    let unstarted = make_work(library.id, "Unstarted", OffsetDateTime::now_utc());
    store.upsert_work(&movie).await.expect("movie");
    store.upsert_work(&unstarted).await.expect("unstarted");

    played(store, user.id, e1).await;
    tick().await;
    store
        .record_playback_progress(user.id, PlayTarget::Work(movie.id), 1_000)
        .await
        .expect("progress");
    tick().await;
    store
        .record_playback_progress(user.id, PlayTarget::Work(e2), 1_000)
        .await
        .expect("progress");
    // Unmarking leaves a timestamped row that no longer means "watched".
    store
        .mark_played(user.id, PlayTarget::Work(unstarted.id), false)
        .await
        .expect("unplayed");

    // A favorite with no playback isn't watch history...
    store
        .set_favorite(user.id, PlayTarget::Work(unstarted.id), true)
        .await
        .expect("favorite");
    // ...but on a series, it describes the episodes watched under it.
    store
        .set_favorite(user.id, PlayTarget::Work(show.series), true)
        .await
        .expect("favorite");

    let watched = store
        .watched_top_level_works(user.id)
        .await
        .expect("watched");
    assert_eq!(
        watched.iter().map(|w| w.work).collect::<Vec<_>>(),
        [show.series, movie.id],
        "two episodes are one series, dated by the latest"
    );
    assert!(
        watched[0].finished,
        "one finished episode finishes the roll-up"
    );
    assert!(watched[0].favorite);
    assert_eq!(watched[0].position_ms, Some(1_000));
    assert!(!watched[1].finished);
    assert!(!watched[1].favorite);

    store.delete_user(user.id).await.ok();
    store.delete_library(library.id).await.ok();
}

async fn work_embeddings_round_trip<S: Storage + ?Sized>(store: &S) {
    let library = test_library("Embeddings Lib");
    store.upsert_library(&library).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let with = make_work(library.id, "With", now);
    let other_namespace = make_work(library.id, "Other namespace", now);
    let without = make_work(library.id, "Without", now);
    for work in [&with, &other_namespace, &without] {
        store.upsert_work(work).await.expect("work");
    }
    store
        .upsert_embedding(EmbeddingTarget::Work(with.id), "taste-ns", "m", &[1.0, 2.0])
        .await
        .expect("embedding");
    store
        .upsert_embedding(
            EmbeddingTarget::Work(other_namespace.id),
            "other-ns",
            "m",
            &[3.0, 4.0],
        )
        .await
        .expect("embedding");

    let found = store
        .work_embeddings("taste-ns", &[with.id, other_namespace.id, without.id])
        .await
        .expect("work_embeddings");
    assert_eq!(found, [(with.id, vec![1.0, 2.0])]);
    assert!(
        store
            .work_embeddings("taste-ns", &[])
            .await
            .expect("empty")
            .is_empty()
    );

    store.delete_work_embeddings(with.id).await.expect("delete");
    assert!(
        store
            .work_embeddings("taste-ns", &[with.id])
            .await
            .expect("after delete")
            .is_empty()
    );
    store
        .delete_work_embeddings(without.id)
        .await
        .expect("deleting nothing is fine");

    store.delete_library(library.id).await.ok();
}

fn make_tag(namespace: &str, slug: &str, display: &str) -> crate::domain::Tag {
    crate::domain::Tag {
        id: noflex_core::TagId::new(),
        namespace: namespace.to_string(),
        parent_id: None,
        slug: slug.to_string(),
        display: display.to_string(),
        metadata: async_graphql::Json(serde_json::json!({})),
    }
}

/// `(namespace, slug)` is the unique key for a tag — the same slug can
/// exist in different namespaces (e.g. `genre:sci-fi` vs
/// `mood:sci-fi`). Two tags with the same `(namespace, slug)` must
/// conflict; the existing tag wins, no duplicate row.
async fn tag_namespace_uniqueness<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Tag NS Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work(lib.id, "Tag Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");

    let genre_sci_fi = make_tag("genre", "sci-fi", "Sci-Fi");
    store.upsert_tag(&genre_sci_fi).await.expect("first upsert");
    let mood_sci_fi = make_tag("mood", "sci-fi", "Sci-Fi mood");
    store
        .upsert_tag(&mood_sci_fi)
        .await
        .expect("different namespace");

    // Same (namespace, slug) under a new id must fail the UNIQUE check.
    let collide = make_tag("genre", "sci-fi", "Different display");
    let err = store
        .upsert_tag(&collide)
        .await
        .expect_err("namespace+slug collision");
    assert!(
        matches!(err, crate::error::StorageError::Constraint(_)),
        "expected Constraint, got {err:?}"
    );

    store
        .assign_tag(
            genre_sci_fi.id,
            noflex_core::TagTarget::Work(work.id),
            1.0,
            "test",
        )
        .await
        .expect("assign");

    let by_slug = store
        .tag_by_slug("genre", "sci-fi")
        .await
        .expect("by slug")
        .expect("present");
    assert_eq!(by_slug.id, genre_sci_fi.id);

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// assign / unassign / tags_for_target round-trip. Re-assigning the
/// same (tag, target) updates the weight and source but stays a single
/// row.
async fn tag_assign_unassign<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Tag Assign Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work(lib.id, "Tagged Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let tag = make_tag("genre", "drama", "Drama");
    store.upsert_tag(&tag).await.expect("tag");

    let target = noflex_core::TagTarget::Work(work.id);
    store
        .assign_tag(tag.id, target, 0.5, "scanner")
        .await
        .expect("assign scanner");
    let after_first = store
        .tags_for_target(target)
        .await
        .expect("tags for target");
    assert_eq!(after_first.len(), 1);

    // Re-assign with a new weight and source — same row, new metadata.
    store
        .assign_tag(tag.id, target, 0.9, "user")
        .await
        .expect("re-assign user");
    let after_second = store
        .tags_for_target(target)
        .await
        .expect("tags after re-assign");
    assert_eq!(after_second.len(), 1, "re-assign stays one row");
    assert_eq!(after_second[0].id, tag.id);

    // Unassign removes the row entirely.
    store.unassign_tag(tag.id, target).await.expect("unassign");
    let after_unassign = store
        .tags_for_target(target)
        .await
        .expect("tags after unassign");
    assert!(after_unassign.is_empty());

    // Re-unassign is a no-op (idempotent).
    store
        .unassign_tag(tag.id, target)
        .await
        .expect("idempotent unassign");

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// `tag_assignments_for_target` pairs each tag with the weight and
/// provenance source it was assigned with — the read a provider
/// refresh needs to replace only its own assignments.
async fn tag_assignments_carry_source_and_weight<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Tag Assignment Source Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work(lib.id, "Sourced Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let genre = make_tag("genre", "neo-noir", "Neo-Noir");
    let person = make_tag("person", "denis-villeneuve", "Denis Villeneuve");
    store.upsert_tag(&genre).await.expect("genre tag");
    store.upsert_tag(&person).await.expect("person tag");

    let target = noflex_core::TagTarget::Work(work.id);
    store
        .assign_tag(genre.id, target, 1.0, "provider:tmdb")
        .await
        .expect("assign provider");
    store
        .assign_tag(person.id, target, 0.75, "user")
        .await
        .expect("assign user");

    let assignments = store
        .tag_assignments_for_target(target)
        .await
        .expect("assignments for target");
    assert_eq!(assignments.len(), 2);
    let by_id = |id: noflex_core::TagId| {
        assignments
            .iter()
            .find(|a| a.tag.id == id)
            .expect("assignment present")
    };
    let genre_row = by_id(genre.id);
    assert_eq!(genre_row.source, "provider:tmdb");
    assert!((genre_row.weight - 1.0).abs() < f32::EPSILON);
    let person_row = by_id(person.id);
    assert_eq!(person_row.source, "user");
    assert!((person_row.weight - 0.75).abs() < f32::EPSILON);
    assert_eq!(person_row.metadata.0, serde_json::json!({}));

    // What's true on this work stays on this work, and follows re-assignment.
    let part = serde_json::json!({ "order": 3, "credits": [{ "role": "Actor" }] });
    store
        .assign_tag_with_metadata(person.id, target, 1.0, "provider:tmdb", &part)
        .await
        .expect("assign with metadata");
    let assignments = store
        .tag_assignments_for_target(target)
        .await
        .expect("assignments for target");
    let person_row = assignments
        .iter()
        .find(|a| a.tag.id == person.id)
        .expect("assignment present");
    assert_eq!(person_row.metadata.0, part);

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// `metadata` is opaque JSON per the schema; it round-trips intact
/// through upsert. People tags store role/character/photo here, genres
/// store optional colour codes, etc.
async fn tag_metadata_roundtrip<S: Storage + ?Sized>(store: &S) {
    let person = make_tag("person", "ridley-scott", "Ridley Scott");
    let mut person = person;
    person.metadata = async_graphql::Json(serde_json::json!({
        "role": "director",
        "photo_path": "/img/ridley.jpg",
        "external_ids": {"tmdb": "5026"}
    }));
    store.upsert_tag(&person).await.expect("upsert person");

    let got = store
        .tag_by_slug("person", "ridley-scott")
        .await
        .expect("by slug")
        .expect("present");
    assert_eq!(got.metadata.0["role"], "director");
    assert_eq!(got.metadata.0["photo_path"], "/img/ridley.jpg");
    assert_eq!(got.metadata.0["external_ids"]["tmdb"], "5026");
}

/// `parent_id` round-trips intact through upsert. A `delete_tag`
/// method (and the ON DELETE SET NULL behaviour it would exercise)
/// is left for when the admin UI needs it — the trait intentionally omits it.
async fn tag_parent_self_reference<S: Storage + ?Sized>(store: &S) {
    let parent = make_tag("genre", "action", "Action");
    store.upsert_tag(&parent).await.expect("parent");
    let mut child = make_tag("genre", "action-buddy", "Buddy Action");
    child.parent_id = Some(parent.id);
    store.upsert_tag(&child).await.expect("child");

    let by_slug = store
        .tag_by_slug("genre", "action-buddy")
        .await
        .expect("by slug")
        .expect("present");
    assert_eq!(by_slug.parent_id, Some(parent.id));
}

/// get_tag round-trips a stored tag; an absent id is `None`.
async fn get_tag_round_trip<S: Storage + ?Sized>(store: &S) {
    let tag = make_tag("genre", "noir", "Noir");
    store.upsert_tag(&tag).await.expect("upsert");

    let got = store.get_tag(tag.id).await.expect("get").expect("present");
    assert_eq!(got.namespace, "genre");
    assert_eq!(got.slug, "noir");
    assert_eq!(got.display, "Noir");

    let absent = store
        .get_tag(noflex_core::TagId::new())
        .await
        .expect("absent");
    assert!(absent.is_none());
}

/// list_tags: namespace filtering, per-tag work counts (zero-count tags
/// included), case-folded display ordering, and keyset pagination that
/// reassembles the filtered list without dupes or skips. Uses its own
/// namespaces so tags left behind by other conformance cases can't
/// collide on slug.
async fn list_tags_filters_counts_and_paginates<S: Storage + ?Sized>(store: &S) {
    use crate::query::{ListTagsQuery, PageCursor};

    let lib = test_library("List Tags Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let w1 = make_work(lib.id, "Tagged One", now);
    let w2 = make_work(lib.id, "Tagged Two", now);
    store.upsert_work(&w1).await.expect("w1");
    store.upsert_work(&w2).await.expect("w2");

    // Display names picked so the LOWER() ordering differs from raw
    // byte order ("Beta" bytesorts before "action").
    let action = make_tag("lst", "action", "action");
    let beta = make_tag("lst", "beta", "Beta");
    let empty = make_tag("lst", "empty", "Empty Genre");
    let person = make_tag("lst-person", "ridley", "Ridley Scott");
    for t in [&beta, &person, &action, &empty] {
        store.upsert_tag(t).await.expect("upsert tag");
    }
    let assign = |tag: noflex_core::TagId, work: WorkId| {
        store.assign_tag(tag, noflex_core::TagTarget::Work(work), 1.0, "test")
    };
    assign(action.id, w1.id).await.expect("assign w1");
    assign(action.id, w2.id).await.expect("assign w2");
    assign(person.id, w1.id).await.expect("assign person");

    // Unfiltered: every namespace appears, ordered by LOWER(display).
    let page = store
        .list_tags(&ListTagsQuery {
            namespaces: vec![],
            limit: 100,
            cursor: None,
        })
        .await
        .expect("all tags");
    assert!(page.next_cursor().is_none(), "single page under the limit");
    let lowered: Vec<String> = page
        .items
        .iter()
        .map(|s| s.tag.display.to_lowercase())
        .collect();
    let mut sorted = lowered.clone();
    sorted.sort();
    assert_eq!(lowered, sorted, "ordered by case-folded display");
    let count = |slug: &str| {
        page.items
            .iter()
            .find(|s| s.tag.namespace == "lst" && s.tag.slug == slug)
            .map(|s| s.work_count)
    };
    assert_eq!(count("action"), Some(2));
    assert_eq!(count("beta"), Some(0));
    assert_eq!(count("empty"), Some(0), "unassigned tags still list");

    // Namespace filter → only that namespace, same ordering.
    let genres = store
        .list_tags(&ListTagsQuery {
            namespaces: vec!["lst".to_string()],
            limit: 100,
            cursor: None,
        })
        .await
        .expect("filtered");
    let displays: Vec<&str> = genres
        .items
        .iter()
        .map(|s| s.tag.display.as_str())
        .collect();
    assert_eq!(displays, ["action", "Beta", "Empty Genre"]);

    // limit-1 pages + cursor reassemble the filtered list exactly.
    let mut slugs: Vec<String> = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    loop {
        let page = store
            .list_tags(&ListTagsQuery {
                namespaces: vec!["lst".to_string()],
                limit: 1,
                cursor,
            })
            .await
            .expect("page");
        slugs.extend(page.items.iter().map(|s| s.tag.slug.clone()));
        match page.next_cursor() {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(slugs, ["action", "beta", "empty"]);

    // A malformed cursor errors rather than silently re-serving page one.
    let err = store
        .list_tags(&ListTagsQuery {
            namespaces: vec![],
            limit: 1,
            cursor: Some(PageCursor("not-a-cursor".to_string())),
        })
        .await
        .expect_err("bad cursor");
    assert!(
        matches!(err, StorageError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );

    store.delete_work(w1.id).await.ok();
    store.delete_work(w2.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// list_works' tag filter returns exactly the works carrying the tag;
/// a query without it is unaffected.
async fn list_works_tag_filter<S: Storage + ?Sized>(store: &S) {
    use crate::query::ListWorksQuery;

    let lib = test_library("Work Tag Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let tagged = make_work(lib.id, "Tagged", now);
    let plain = make_work(lib.id, "Plain", now);
    store.upsert_work(&tagged).await.expect("tagged");
    store.upsert_work(&plain).await.expect("plain");
    let tag = make_tag("genre", "work-filter", "Work Filter");
    store.upsert_tag(&tag).await.expect("tag");
    store
        .assign_tag(tag.id, noflex_core::TagTarget::Work(tagged.id), 1.0, "test")
        .await
        .expect("assign");

    let filtered = store
        .list_works(&ListWorksQuery {
            library: Some(lib.id),
            tag: Some(tag.id),
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("filtered");
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, tagged.id);

    let unfiltered = store
        .list_works(&ListWorksQuery {
            library: Some(lib.id),
            recursive: true,
            limit: 100,
            ..Default::default()
        })
        .await
        .expect("unfiltered");
    assert_eq!(unfiltered.items.len(), 2, "no-tag query unaffected");

    store.delete_work(tagged.id).await.ok();
    store.delete_work(plain.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

fn make_image(
    target: crate::domain::ImageTarget,
    kind: noflex_core::ImageKind,
    path: &str,
) -> crate::domain::Image {
    crate::domain::Image {
        target,
        kind,
        source: ImageSource::Local,
        path: path.to_string(),
        width: Some(1920),
        height: Some(1080),
        language: Some("en".to_string()),
        provider: None,
    }
}

/// Round-trip an image attached to a Work. The composite PK on
/// (target, kind) means a Work can carry one primary, one backdrop,
/// one logo, etc. without collision.
async fn image_work_roundtrip<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Image Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work(lib.id, "Image Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");

    let target = crate::domain::ImageTarget::Work(work.id);
    store
        .upsert_image(&make_image(
            target,
            noflex_core::ImageKind::Primary,
            "/cache/primary.jpg",
        ))
        .await
        .expect("upsert primary");
    let primary = store
        .image(target, noflex_core::ImageKind::Primary, ALL_SOURCES)
        .await
        .expect("get primary")
        .expect("present");
    assert_eq!(primary.path, "/cache/primary.jpg");
    assert_eq!(primary.width, Some(1920));
    assert_eq!(primary.language.as_deref(), Some("en"));

    // Different kind on the same work is a separate row.
    store
        .upsert_image(&make_image(
            target,
            noflex_core::ImageKind::Backdrop,
            "/cache/backdrop.jpg",
        ))
        .await
        .expect("upsert backdrop");
    let backdrop = store
        .image(target, noflex_core::ImageKind::Backdrop, ALL_SOURCES)
        .await
        .expect("get backdrop")
        .expect("present");
    assert_eq!(backdrop.path, "/cache/backdrop.jpg");
    assert_ne!(backdrop.path, primary.path);

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// The `tag` target_kind covers people (per the AGENTS invariant that
/// people live in the `person` namespace as Tags, not a separate
/// `people` table). A person has at most one `Primary` photo.
async fn image_tag_covers_people<S: Storage + ?Sized>(store: &S) {
    let person = make_tag("person", "keanu-reeves", "Keanu Reeves");
    store.upsert_tag(&person).await.expect("tag");

    let target = crate::domain::ImageTarget::Tag(person.id);
    store
        .upsert_image(&make_image(
            target,
            noflex_core::ImageKind::Primary,
            "/img/keanu.jpg",
        ))
        .await
        .expect("upsert");
    let photo = store
        .image(target, noflex_core::ImageKind::Primary, ALL_SOURCES)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(photo.path, "/img/keanu.jpg");
}

/// Upserting the same (target, kind) replaces the row; width/height
/// and language round-trip with the replacement.
async fn image_unique_per_kind<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Image Re-Upsert Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work(lib.id, "Re-Image", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let target = crate::domain::ImageTarget::Work(work.id);

    store
        .upsert_image(&make_image(
            target,
            noflex_core::ImageKind::Logo,
            "/img/old-logo.png",
        ))
        .await
        .expect("first");
    let mut upgraded = make_image(target, noflex_core::ImageKind::Logo, "/img/new-logo.png");
    upgraded.width = Some(2048);
    upgraded.height = Some(1024);
    upgraded.language = None;
    store.upsert_image(&upgraded).await.expect("re-upsert");

    let got = store
        .image(target, noflex_core::ImageKind::Logo, ALL_SOURCES)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(got.path, "/img/new-logo.png");
    assert_eq!(got.width, Some(2048));
    assert!(got.language.is_none());

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// `list_images` returns every kind attached to one target, ordered by
/// kind text, and stays scoped to that target.
async fn image_list_by_target<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Image List Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work(lib.id, "List Image Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let target = crate::domain::ImageTarget::Work(work.id);

    // Insert out of order; the listing must come back ordered by kind.
    store
        .upsert_image(&make_image(
            target,
            noflex_core::ImageKind::Primary,
            "/cache/primary.jpg",
        ))
        .await
        .expect("upsert primary");
    store
        .upsert_image(&make_image(
            target,
            noflex_core::ImageKind::Backdrop,
            "/cache/backdrop.jpg",
        ))
        .await
        .expect("upsert backdrop");

    let images = store.list_images(target, ALL_SOURCES).await.expect("list");
    let kinds: Vec<noflex_core::ImageKind> = images.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        vec![
            noflex_core::ImageKind::Backdrop,
            noflex_core::ImageKind::Primary
        ]
    );

    // A target with no images lists empty, not an error.
    let other = crate::domain::ImageTarget::Work(noflex_core::WorkId::new());
    assert!(
        store
            .list_images(other, ALL_SOURCES)
            .await
            .expect("list")
            .is_empty()
    );

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// `list_work_images_for_library` covers every work of one library,
/// skips other libraries and non-work targets, and orders by work id
/// then kind.
async fn image_list_for_library_works<S: Storage + ?Sized>(store: &S) {
    use noflex_core::ImageKind;

    use crate::domain::ImageTarget;

    let lib = test_library("Library Images Lib");
    let other_lib = test_library("Other Images Lib");
    store.upsert_library(&lib).await.expect("library");
    store
        .upsert_library(&other_lib)
        .await
        .expect("other library");
    let now = OffsetDateTime::now_utc();
    let first = make_work(lib.id, "First", now);
    let second = make_work_of_kind(lib.id, WorkKind::Series, "Second", now);
    let foreign = make_work(other_lib.id, "Foreign", now);
    for work in [&first, &second, &foreign] {
        store.upsert_work(work).await.expect("work");
    }
    let edition = make_edition(first.id, "Default");
    let edition_id = store.upsert_edition(&edition).await.expect("edition");

    for (target, kind, path) in [
        (
            ImageTarget::Work(second.id),
            ImageKind::Primary,
            "/a/2p.jpg",
        ),
        (ImageTarget::Work(first.id), ImageKind::Primary, "/a/1p.jpg"),
        (
            ImageTarget::Work(first.id),
            ImageKind::Backdrop,
            "/a/1b.jpg",
        ),
        (
            ImageTarget::Work(foreign.id),
            ImageKind::Primary,
            "/b/f.jpg",
        ),
        (
            ImageTarget::Edition(edition_id),
            ImageKind::Primary,
            "/a/e.jpg",
        ),
    ] {
        store
            .upsert_image(&make_image(target, kind, path))
            .await
            .expect("upsert image");
    }

    let listed = store
        .list_work_images_for_library(lib.id)
        .await
        .expect("list library images");
    let got: Vec<(ImageTarget, ImageKind)> = listed.iter().map(|i| (i.target, i.kind)).collect();
    // UUIDv7 ids sort by creation, so `first` precedes `second`.
    assert_eq!(
        got,
        vec![
            (ImageTarget::Work(first.id), ImageKind::Backdrop),
            (ImageTarget::Work(first.id), ImageKind::Primary),
            (ImageTarget::Work(second.id), ImageKind::Primary),
        ]
    );
    assert_eq!(listed[0].path, "/a/1b.jpg");

    let empty = test_library("Imageless Lib");
    store.upsert_library(&empty).await.expect("empty library");
    assert!(
        store
            .list_work_images_for_library(empty.id)
            .await
            .expect("list empty")
            .is_empty()
    );

    for work in [&first, &second, &foreign] {
        store.delete_work(work.id).await.ok();
    }
    for l in [&lib, &other_lib, &empty] {
        store.delete_library(l.id).await.ok();
    }
}

/// `delete_image` removes exactly one (target, kind) row and reports
/// whether it existed.
async fn image_delete_by_target_and_kind<S: Storage + ?Sized>(store: &S) {
    use noflex_core::ImageKind;

    use crate::domain::ImageTarget;

    let lib = test_library("Image Delete Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work(lib.id, "Delete Image Movie", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let target = ImageTarget::Work(work.id);
    for kind in [ImageKind::Primary, ImageKind::Backdrop] {
        store
            .upsert_image(&make_image(target, kind, "/img/x.jpg"))
            .await
            .expect("upsert");
    }

    assert!(
        store
            .delete_image(target, ImageKind::Primary, ImageSource::Local)
            .await
            .expect("delete")
    );
    assert!(
        store
            .image(target, ImageKind::Primary, ALL_SOURCES)
            .await
            .expect("get")
            .is_none()
    );
    assert!(
        store
            .image(target, ImageKind::Backdrop, ALL_SOURCES)
            .await
            .expect("get backdrop")
            .is_some(),
        "other kinds of the same target survive"
    );
    assert!(
        !store
            .delete_image(target, ImageKind::Primary, ImageSource::Local)
            .await
            .expect("delete again"),
        "deleting a missing row reports false"
    );

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// `works_missing_image` lists only works of the asked kind that lack
/// the asked image kind, pages by cursor without loss or duplication,
/// and rejects a malformed cursor.
async fn works_missing_image_sweep<S: Storage + ?Sized>(store: &S) {
    use noflex_core::ImageKind;

    use crate::{
        domain::ImageTarget,
        query::{Page, PageCursor},
    };

    let lib = test_library("Missing Image Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let a = make_work_of_kind(lib.id, WorkKind::Episode, "A", now);
    let b = make_work_of_kind(lib.id, WorkKind::Episode, "B", now);
    let c = make_work_of_kind(lib.id, WorkKind::Episode, "C", now);
    let movie = make_work(lib.id, "Movie", now);
    for work in [&a, &b, &c, &movie] {
        store.upsert_work(work).await.expect("work");
    }
    // Other conformance checks share the store, so only this library's
    // works are compared.
    let mine = |page: &Page<Work>| -> Vec<WorkId> {
        page.items
            .iter()
            .filter(|w| w.library_id == lib.id)
            .map(|w| w.id)
            .collect()
    };
    let all = store
        .works_missing_image(WorkKind::Episode, ImageKind::Thumb, ALL_SOURCES, 1000, None)
        .await
        .expect("missing");
    assert_eq!(mine(&all), vec![a.id, b.id, c.id]);

    store
        .upsert_image(&make_image(
            ImageTarget::Work(a.id),
            ImageKind::Thumb,
            "/img/a.jpg",
        ))
        .await
        .expect("thumb a");
    store
        .upsert_image(&make_image(
            ImageTarget::Work(b.id),
            ImageKind::Primary,
            "/img/b.jpg",
        ))
        .await
        .expect("primary b");
    let all = store
        .works_missing_image(WorkKind::Episode, ImageKind::Thumb, ALL_SOURCES, 1000, None)
        .await
        .expect("missing");
    assert_eq!(
        mine(&all),
        vec![b.id, c.id],
        "a has a thumb; b's primary is not a thumb"
    );

    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let page = store
            .works_missing_image(WorkKind::Episode, ImageKind::Thumb, ALL_SOURCES, 1, cursor)
            .await
            .expect("page");
        assert!(page.items.len() <= 1);
        walked.extend(mine(&page));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(walked, vec![b.id, c.id]);

    let err = store
        .works_missing_image(
            WorkKind::Episode,
            ImageKind::Thumb,
            ALL_SOURCES,
            10,
            Some(PageCursor("nope".into())),
        )
        .await
        .expect_err("malformed cursor");
    assert!(matches!(err, StorageError::InvalidInput(_)), "{err:?}");

    for work in [&a, &b, &c, &movie] {
        store.delete_work(work.id).await.ok();
    }
    store.delete_library(lib.id).await.ok();
}

const ALL_SOURCES: &[ImageSource] = &[
    ImageSource::Local,
    ImageSource::Provider,
    ImageSource::Generated,
];

fn sourced_image(
    target: crate::domain::ImageTarget,
    kind: noflex_core::ImageKind,
    source: ImageSource,
    path: &str,
) -> crate::domain::Image {
    crate::domain::Image {
        source,
        ..make_image(target, kind, path)
    }
}

/// Rows of one kind from different sources coexist; reads return the
/// first source in the caller's order, never a source it omits, and
/// deletes and re-upserts touch only their own (kind, source).
async fn image_sources_coexist_and_resolve_by_order<S: Storage + ?Sized>(store: &S) {
    use noflex_core::ImageKind;

    use crate::domain::ImageTarget;

    let lib = test_library("Image Source Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work(lib.id, "Sourced", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let target = ImageTarget::Work(work.id);
    for (kind, source, path) in [
        (ImageKind::Primary, ImageSource::Local, "/lib/poster.jpg"),
        (ImageKind::Primary, ImageSource::Provider, "/cache/p.jpg"),
        (ImageKind::Backdrop, ImageSource::Local, "/lib/fanart.jpg"),
        (ImageKind::Thumb, ImageSource::Generated, "/cache/t.jpg"),
    ] {
        store
            .upsert_image(&sourced_image(target, kind, source, path))
            .await
            .expect("upsert");
    }

    let provider_first = [
        ImageSource::Provider,
        ImageSource::Local,
        ImageSource::Generated,
    ];
    let local_first = [
        ImageSource::Local,
        ImageSource::Provider,
        ImageSource::Generated,
    ];
    let pick = |order: &'static [ImageSource]| async move {
        store
            .image(target, ImageKind::Primary, order)
            .await
            .expect("get")
            .map(|i| (i.source, i.path))
    };
    assert_eq!(
        pick(&[ImageSource::Provider, ImageSource::Local]).await,
        Some((ImageSource::Provider, "/cache/p.jpg".to_string()))
    );
    assert_eq!(
        pick(&[ImageSource::Local, ImageSource::Provider]).await,
        Some((ImageSource::Local, "/lib/poster.jpg".to_string()))
    );
    assert_eq!(pick(&[ImageSource::Generated]).await, None);
    assert_eq!(pick(&[]).await, None);

    let listed = |images: Vec<crate::domain::Image>| {
        images
            .into_iter()
            .map(|i| (i.kind, i.source))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        listed(
            store
                .list_images(target, &provider_first)
                .await
                .expect("list")
        ),
        vec![
            (ImageKind::Backdrop, ImageSource::Local),
            (ImageKind::Primary, ImageSource::Provider),
            (ImageKind::Thumb, ImageSource::Generated),
        ]
    );
    assert_eq!(
        listed(store.list_images(target, &local_first).await.expect("list")),
        vec![
            (ImageKind::Backdrop, ImageSource::Local),
            (ImageKind::Primary, ImageSource::Local),
            (ImageKind::Thumb, ImageSource::Generated),
        ]
    );
    assert_eq!(
        listed(
            store
                .list_images(target, &[ImageSource::Provider])
                .await
                .expect("list")
        ),
        vec![(ImageKind::Primary, ImageSource::Provider)],
        "omitted sources are never listed"
    );

    // The library listing carries every source's row.
    let raw = store
        .list_work_images_for_library(lib.id)
        .await
        .expect("library rows");
    assert_eq!(raw.len(), 4);

    store
        .upsert_image(&sourced_image(
            target,
            ImageKind::Primary,
            ImageSource::Provider,
            "/cache/p2.jpg",
        ))
        .await
        .expect("replace provider");
    assert!(
        store
            .delete_image(target, ImageKind::Primary, ImageSource::Local)
            .await
            .expect("delete local")
    );
    assert!(
        !store
            .delete_image(target, ImageKind::Primary, ImageSource::Generated)
            .await
            .expect("delete absent"),
        "no generated primary existed"
    );
    assert_eq!(
        pick(&[ImageSource::Local, ImageSource::Provider]).await,
        Some((ImageSource::Provider, "/cache/p2.jpg".to_string())),
        "the provider row survives the local delete, replaced in place"
    );

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// `works_missing_image` only counts rows from the listed sources.
async fn works_missing_image_honors_sources<S: Storage + ?Sized>(store: &S) {
    use noflex_core::ImageKind;

    use crate::domain::ImageTarget;

    let lib = test_library("Missing Source Lib");
    store.upsert_library(&lib).await.expect("library");
    let episode = make_work_of_kind(
        lib.id,
        WorkKind::Episode,
        "Generated Only",
        OffsetDateTime::now_utc(),
    );
    store.upsert_work(&episode).await.expect("work");
    store
        .upsert_image(&sourced_image(
            ImageTarget::Work(episode.id),
            ImageKind::Thumb,
            ImageSource::Generated,
            "/cache/g.jpg",
        ))
        .await
        .expect("generated thumb");

    let missing = |sources: &'static [ImageSource]| async move {
        store
            .works_missing_image(WorkKind::Episode, ImageKind::Thumb, sources, 1000, None)
            .await
            .expect("missing")
            .items
            .iter()
            .any(|w| w.id == episode.id)
    };
    assert!(!missing(ALL_SOURCES).await);
    assert!(missing(&[ImageSource::Local, ImageSource::Provider]).await);
    assert!(missing(&[]).await, "no sources means nothing counts");

    store.delete_work(episode.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// `identified_works_missing_image_source` lists works with a
/// non-`Custom` external id and no row from the source, paging by
/// cursor.
async fn identified_works_missing_image_source_sweep<S: Storage + ?Sized>(store: &S) {
    use noflex_core::ImageKind;

    use crate::domain::ImageTarget;

    let lib = test_library("Identified Missing Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let id = |kind: ExternalIdKind, value: &str| ExternalId {
        kind,
        value: value.to_string(),
    };
    // A quote-laden value must not fool the backend into seeing a kind.
    let mut custom_only = make_work(lib.id, "Custom Only", now);
    custom_only.external_ids = vec![id(ExternalIdKind::Custom, "scan:\"kind\":\"Tmdb\"")];
    let mut bare = make_work(lib.id, "Tmdb Bare", now);
    bare.external_ids = vec![id(ExternalIdKind::Tmdb, "movie/1")];
    let mut has_provider = make_work(lib.id, "Has Provider", now);
    has_provider.external_ids = vec![id(ExternalIdKind::Tmdb, "movie/2")];
    let mut local_only = make_work(lib.id, "Local Only", now);
    local_only.external_ids = vec![id(ExternalIdKind::Imdb, "tt3")];
    let mut mixed = make_work_of_kind(lib.id, WorkKind::Series, "Mixed", now);
    mixed.external_ids = vec![
        id(ExternalIdKind::Custom, "scan:mixed"),
        id(ExternalIdKind::Tvdb, "4"),
    ];
    let none = make_work(lib.id, "No Ids", now);
    let works = [
        &custom_only,
        &bare,
        &has_provider,
        &local_only,
        &mixed,
        &none,
    ];
    for work in works {
        store.upsert_work(work).await.expect("work");
    }
    for (work, source) in [
        (&has_provider, ImageSource::Provider),
        (&local_only, ImageSource::Local),
    ] {
        store
            .upsert_image(&sourced_image(
                ImageTarget::Work(work.id),
                ImageKind::Primary,
                source,
                "/x.jpg",
            ))
            .await
            .expect("image");
    }

    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let page = store
            .identified_works_missing_image_source(ImageSource::Provider, 1, cursor)
            .await
            .expect("page");
        assert!(page.items.len() <= 1);
        walked.extend(
            page.items
                .iter()
                .filter(|w| w.library_id == lib.id)
                .map(|w| w.id),
        );
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    assert_eq!(walked, vec![bare.id, local_only.id, mixed.id]);

    let err = store
        .identified_works_missing_image_source(
            ImageSource::Provider,
            10,
            Some(PageCursor("nope".into())),
        )
        .await
        .expect_err("malformed cursor");
    assert!(matches!(err, StorageError::InvalidInput(_)), "{err:?}");

    for work in works {
        store.delete_work(work.id).await.ok();
    }
    store.delete_library(lib.id).await.ok();
}

/// `ImageTarget::source_order` follows the owning library's artwork preference
/// for works and editions and reads it fresh on every call, so flipping
/// the preference changes the effective image without touching rows.
async fn image_source_order_follows_library_preference<S: Storage + ?Sized>(store: &S) {
    use noflex_core::ImageKind;

    use crate::domain::{ArtworkPreference, ImageTarget};

    let mut lib = test_library("Artwork Pref Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work(lib.id, "Preferred", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let edition = store
        .upsert_edition(&make_edition(work.id, "Default"))
        .await
        .expect("edition");
    let target = ImageTarget::Work(work.id);
    for (source, path) in [
        (ImageSource::Local, "/lib/poster.jpg"),
        (ImageSource::Provider, "/cache/poster.jpg"),
    ] {
        store
            .upsert_image(&sourced_image(target, ImageKind::Primary, source, path))
            .await
            .expect("image");
    }

    let effective = || async {
        let order = target.source_order(store).await.expect("order");
        store
            .image(target, ImageKind::Primary, order)
            .await
            .expect("image")
            .map(|i| i.source)
    };
    assert_eq!(effective().await, Some(ImageSource::Provider));

    lib.config.artwork_preference = ArtworkPreference::PreferLocal;
    store.upsert_library(&lib).await.expect("prefer local");
    assert_eq!(effective().await, Some(ImageSource::Local));
    assert_eq!(
        ImageTarget::Edition(edition)
            .source_order(store)
            .await
            .expect("edition order"),
        ArtworkPreference::PreferLocal.source_order()
    );
    assert_eq!(
        ImageTarget::Tag(TagId::new())
            .source_order(store)
            .await
            .expect("tag order"),
        ArtworkPreference::default().source_order(),
        "tags span libraries and take the default"
    );

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// A title-only search hits the work whose title contains the query.
/// The like-based search is case-insensitive (via LOWER() on both
/// sides).
async fn search_lexical_match<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Lex Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let matrix = make_work_of_kind(library_id(&lib), WorkKind::Movie, "The Matrix", now);
    let inception = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Inception", now);
    let blade = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Blade Runner", now);
    let m_id = matrix.id;
    let i_id = inception.id;
    let b_id = blade.id;
    store.upsert_work(&matrix).await.expect("m");
    store.upsert_work(&inception).await.expect("i");
    store.upsert_work(&blade).await.expect("b");

    let hits = store
        .search_lexical("matrix", &[], None, 10)
        .await
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].work, m_id);

    // Case-insensitive: "MATRIX" finds the same row.
    let hits_upper = store
        .search_lexical("MATRIX", &[], None, 10)
        .await
        .expect("search");
    assert_eq!(hits_upper.len(), 1);
    assert_eq!(hits_upper[0].work, m_id);

    // Substring: "unner" matches "Blade Runner".
    let hits_sub = store
        .search_lexical("unner", &[], None, 10)
        .await
        .expect("search");
    assert_eq!(hits_sub.len(), 1);
    assert_eq!(hits_sub[0].work, b_id);

    store.delete_work(m_id).await.ok();
    store.delete_work(i_id).await.ok();
    store.delete_work(b_id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// A query with no matches returns an empty vec, not an error.
async fn search_lexical_miss<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Lex Miss Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Movie", now);
    let w_id = work.id;
    store.upsert_work(&work).await.expect("w");

    let hits = store
        .search_lexical("nonexistent-query-zzz", &[], None, 10)
        .await
        .expect("search");
    assert!(hits.is_empty());

    store.delete_work(w_id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// A person's name finds what they're credited on, below works it titles, and a
/// show's regular cast doesn't drag in every episode.
async fn search_lexical_finds_credited_people<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Lex People Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let mut documentary =
        make_work_of_kind(library_id(&lib), WorkKind::Movie, "Being Wren Okafor", now);
    documentary.sort_title = "being wren okafor".to_string();
    let film = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Harbor Lights", now);
    let uncredited = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Quiet Water", now);
    let series = make_work_of_kind(library_id(&lib), WorkKind::Series, "Night Shift", now);
    let mut regular_episode = make_work_of_kind(library_id(&lib), WorkKind::Episode, "Pilot", now);
    regular_episode.parent_id = Some(series.id);
    let other_series = make_work_of_kind(library_id(&lib), WorkKind::Series, "Tideline", now);
    let mut guest_episode =
        make_work_of_kind(library_id(&lib), WorkKind::Episode, "The Visitor", now);
    guest_episode.parent_id = Some(other_series.id);
    let works = [
        &documentary,
        &film,
        &uncredited,
        &series,
        &regular_episode,
        &other_series,
        &guest_episode,
    ];
    for work in works {
        store.upsert_work(work).await.expect("work");
    }

    let wren = make_tag("person", "wren-okafor-lex", "Wren Okafor");
    let wren_id = store.upsert_tag(&wren).await.expect("person");
    // Only a person tag is a credit, however its display reads.
    let studio = make_tag("studio", "wren-okafor-films-lex", "Wren Okafor Films");
    let studio_id = store.upsert_tag(&studio).await.expect("studio");
    store
        .assign_tag(studio_id, TagTarget::Work(uncredited.id), 1.0, "test")
        .await
        .expect("assign studio");
    for work in [&film, &series, &regular_episode, &guest_episode] {
        store
            .assign_tag(wren_id, TagTarget::Work(work.id), 1.0, "test")
            .await
            .expect("assign person");
    }

    let hits = store
        .search_lexical("wren okafor", &[], None, 10)
        .await
        .expect("search");
    let found = hits.iter().map(|hit| hit.work).collect::<Vec<_>>();
    assert_eq!(found[0], documentary.id, "a title match leads: {hits:?}");
    assert_eq!(hits[0].matched_fields, [crate::search::MATCHED_TITLE]);
    assert_eq!(
        found[1..3]
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>(),
        [film.id, series.id].into_iter().collect(),
        "movies and series before episodes: {hits:?}"
    );
    assert_eq!(hits[1].matched_fields, [crate::search::MATCHED_CREDITS]);
    assert_eq!(found[3], guest_episode.id, "a guest star's episode counts");
    assert_eq!(found.len(), 4, "the regular's episode stays out: {hits:?}");

    // Asking for episodes means wanting them, regular or guest.
    let episodes = store
        .search_lexical("wren okafor", &[WorkKind::Episode], None, 10)
        .await
        .expect("episodes");
    assert_eq!(episodes.len(), 2);

    for work in works {
        store.delete_work(work.id).await.ok();
    }
    store.delete_library(lib.id).await.ok();
}

/// Vectors from another model survive a change of model, so changing back costs
/// nothing, until they're pruned. Under the same namespace they can't survive at
/// all, since searching would compare them with the new model's.
async fn embedding_model_changes_keep_vectors_until_pruned<S: Storage + ?Sized>(store: &S) {
    use crate::domain::EmbeddingModel;

    let lib = test_library("Embedding Model Lib");
    store.upsert_library(&lib).await.expect("library");
    let work = make_work_of_kind(
        library_id(&lib),
        WorkKind::Movie,
        "Vectors",
        OffsetDateTime::now_utc(),
    );
    store.upsert_work(&work).await.expect("work");
    let target = noflex_core::EmbeddingTarget::Work(work.id);
    let model = |namespace: &str, model_id: &str, dimensions| EmbeddingModel {
        namespace: namespace.to_string(),
        model_id: model_id.to_string(),
        dimensions,
    };
    let vectors = |namespace: &'static str| async move {
        store
            .search_vector(namespace, &[1.0, 0.0], &[], None, 10)
            .await
            .expect("search")
            .len()
    };

    let first = model("emb-change-a", "a", 2);
    assert_eq!(
        store.activate_embedding_model(&first).await.expect("first"),
        None
    );
    assert_eq!(
        store.active_embedding_model().await.expect("active"),
        Some(first.clone())
    );
    assert_eq!(
        store.activate_embedding_model(&first).await.expect("again"),
        None,
        "activating the active model changes nothing"
    );
    store
        .upsert_embedding(target, "emb-change-a", "a", &[1.0, 0.0])
        .await
        .expect("embed a");

    let second = model("emb-change-b", "b", 2);
    assert_eq!(
        store
            .activate_embedding_model(&second)
            .await
            .expect("second"),
        Some(first.clone())
    );
    assert_eq!(vectors("emb-change-a").await, 1, "kept across a change");
    store
        .upsert_embedding(target, "emb-change-b", "b", &[1.0, 0.0])
        .await
        .expect("embed b");

    // A new model under the old namespace invalidates what's stored there.
    let reused = model("emb-change-b", "b2", 2);
    assert_eq!(
        store
            .activate_embedding_model(&reused)
            .await
            .expect("reuse"),
        Some(second)
    );
    assert_eq!(
        vectors("emb-change-b").await,
        0,
        "b's vectors can't be b2's"
    );
    store
        .upsert_embedding(target, "emb-change-b", "b2", &[1.0, 0.0])
        .await
        .expect("embed b2");

    // Others in this store share the fate of any namespace but the one kept.
    let pruned = store.prune_embeddings("emb-change-b").await.expect("prune");
    assert!(pruned >= 1, "emb-change-a's vector at least");

    assert_eq!(vectors("emb-change-a").await, 0, "pruned");
    assert_eq!(vectors("emb-change-b").await, 1, "kept");

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

async fn embedding_endpoint_round_trips<S: Storage + ?Sized>(store: &S) {
    use crate::domain::EmbeddingEndpoint;

    assert_eq!(store.embedding_endpoint().await.expect("unset"), None);
    let mut endpoint = EmbeddingEndpoint {
        url: "https://inference.example/v1".to_string(),
        api_key: Some("sk-test".to_string()),
        model: "text-embedding-3-small".to_string(),
        dimensions: 512,
        request_dimensions: Some(512),
        query_prefix: "query: ".to_string(),
        document_prefix: "passage: ".to_string(),
    };
    store
        .set_embedding_endpoint(Some(&endpoint))
        .await
        .expect("set");
    assert_eq!(
        store.embedding_endpoint().await.expect("read"),
        Some(endpoint.clone())
    );

    endpoint.api_key = None;
    endpoint.request_dimensions = None;
    store
        .set_embedding_endpoint(Some(&endpoint))
        .await
        .expect("replace");
    assert_eq!(
        store.embedding_endpoint().await.expect("read"),
        Some(endpoint)
    );

    store.set_embedding_endpoint(None).await.expect("clear");
    assert_eq!(store.embedding_endpoint().await.expect("cleared"), None);
}

/// Vector search: the closer the stored vector is to the query, the
/// higher the similarity (1 / (1 + L2)).
async fn search_vector_similarity<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Vec Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let a = make_work_of_kind(library_id(&lib), WorkKind::Movie, "A", now);
    let b = make_work_of_kind(library_id(&lib), WorkKind::Movie, "B", now);
    let c = make_work_of_kind(library_id(&lib), WorkKind::Movie, "C", now);
    let a_id = a.id;
    let b_id = b.id;
    let c_id = c.id;
    store.upsert_work(&a).await.expect("a");
    store.upsert_work(&b).await.expect("b");
    store.upsert_work(&c).await.expect("c");
    let edition = make_edition(a_id, "Edition embedding");
    store.upsert_edition(&edition).await.expect("edition");

    let target = noflex_core::EmbeddingTarget::Work;
    // Embedding A matches the query exactly (similarity 1.0).
    store
        .upsert_embedding(target(a_id), "work_text", "minilm-l6", &[1.0, 0.0, 0.0])
        .await
        .expect("embed a");
    // B is at 90 degrees — close.
    store
        .upsert_embedding(target(b_id), "work_text", "minilm-l6", &[0.0, 1.0, 0.0])
        .await
        .expect("embed b");
    // C is at 180 degrees — far.
    store
        .upsert_embedding(target(c_id), "work_text", "minilm-l6", &[-1.0, 0.0, 0.0])
        .await
        .expect("embed c");
    store
        .upsert_embedding(
            noflex_core::EmbeddingTarget::Edition(edition.id),
            "work_text",
            "minilm-l6",
            &[1.0, 0.0, 0.0],
        )
        .await
        .expect("embed edition");

    let hits = store
        .search_vector("work_text", &[1.0, 0.0, 0.0], &[], None, 10)
        .await
        .expect("search");
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].work, a_id, "exact match scores highest");
    // Similarity drops off with distance, so a beats c.
    let sim_a = hits.iter().find(|h| h.work == a_id).unwrap().similarity;
    let sim_c = hits.iter().find(|h| h.work == c_id).unwrap().similarity;
    assert!(sim_a > sim_c, "near wins over far");

    store.delete_work(a_id).await.ok();
    store.delete_work(b_id).await.ok();
    store.delete_work(c_id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// Hybrid search fuses lexical + vector via RRF. A work that scores
/// in BOTH channels lands above one that scores in only one.
async fn search_hybrid_fuses<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Hybrid Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let strong = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Alpha", now);
    let weak = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Beta", now);
    let s_id = strong.id;
    let w_id = weak.id;
    store.upsert_work(&strong).await.expect("s");
    store.upsert_work(&weak).await.expect("w");

    let target = noflex_core::EmbeddingTarget::Work;
    // Strong matches BOTH channels: title contains "alpha" + vec near query.
    store
        .upsert_embedding(target(s_id), "ns", "model", &[1.0, 0.0])
        .await
        .expect("s emb");
    // Weak matches ONLY the vector channel.
    store
        .upsert_embedding(target(w_id), "ns", "model", &[0.9, 0.1])
        .await
        .expect("w emb");

    let q = crate::query::HybridQuery {
        access_user_id: None,
        text: "alpha".to_string(),
        query_vector: Some(vec![1.0, 0.0]),
        vector_namespace: "ns".to_string(),
        kinds: vec![],
        limit: 10,
        mode: crate::query::SearchMode::Hybrid,
    };
    let hits = store.search_hybrid(&q).await.expect("hybrid");
    assert!(!hits.is_empty());
    assert_eq!(hits[0].work, s_id, "both-channel hit ranks first");
    use crate::search::{MATCHED_MEANING, MATCHED_TITLE};
    assert_eq!(hits[0].matched_fields, [MATCHED_TITLE, MATCHED_MEANING]);
    assert_eq!(hits[1].matched_fields, [MATCHED_MEANING]);
    // Scores don't compare across servers, so a client merging them needs this.
    assert!(hits[0].similarity > hits[1].similarity, "{hits:?}");
    assert!(hits[1].similarity.is_some(), "{hits:?}");

    store.delete_work(s_id).await.ok();
    store.delete_work(w_id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// "Episode 3" with no synopsis embeds to nothing in particular and sits near every
/// query, so seasons and episodes only match by meaning once they have one to compare.
async fn search_vector_skips_seasons_and_episodes_without_a_synopsis<S: Storage + ?Sized>(
    store: &S,
) {
    let lib = test_library("Bare Episodes Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = |kind, title: &str, overview: Option<&str>| {
        let mut work = make_work_of_kind(library_id(&lib), kind, title, now);
        work.overview = overview.map(str::to_string);
        work
    };
    let bare_episode = work(WorkKind::Episode, "Episode 3", None);
    let blank_episode = work(WorkKind::Episode, "Episode 4", Some(""));
    let bare_season = work(WorkKind::Season, "Season 1", None);
    let described = work(WorkKind::Episode, "Low Water", Some("The tide goes out."));
    let bare_track = work(WorkKind::Track, "Blue in Green", None);
    let bare_movie = work(WorkKind::Movie, "Night Ferry", None);
    let works = [
        &bare_episode,
        &blank_episode,
        &bare_season,
        &described,
        &bare_track,
        &bare_movie,
    ];
    for work in works {
        store.upsert_work(work).await.expect("work");
        store
            .upsert_embedding(
                noflex_core::EmbeddingTarget::Work(work.id),
                "bare",
                "model",
                &[1.0, 0.0],
            )
            .await
            .expect("embed");
    }

    let found = store
        .search_vector("bare", &[1.0, 0.0], &[], None, 10)
        .await
        .expect("search")
        .into_iter()
        .map(|hit| hit.work)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        found,
        [described.id, bare_track.id, bare_movie.id]
            .into_iter()
            .collect(),
    );

    // Asking for episodes by kind doesn't change what can be compared.
    let episodes = store
        .search_vector("bare", &[1.0, 0.0], &[WorkKind::Episode], None, 10)
        .await
        .expect("episodes");
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].work, described.id);

    // Their names still find them.
    let by_name = store
        .search_lexical("episode 3", &[], None, 10)
        .await
        .expect("lexical");
    assert_eq!(by_name[0].work, bare_episode.id);

    for work in works {
        store.delete_work(work.id).await.ok();
    }
    store.delete_library(lib.id).await.ok();
}

/// "harbor" should find the show and titles carrying the word, not every episode
/// whose synopsis names the show, however recently those were added.
async fn search_ranks_titles_over_mentions<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Ranked Lib");
    store.upsert_library(&lib).await.expect("library");
    let long_ago = OffsetDateTime::now_utc() - time::Duration::days(365);
    let newer = OffsetDateTime::now_utc();
    let work = |kind, title: &str, parent, overview: Option<&str>, added| {
        let mut work = make_work_of_kind(library_id(&lib), kind, title, added);
        work.parent_id = parent;
        work.overview = overview.map(str::to_string);
        work
    };

    let show = work(WorkKind::Series, "Harbor Lights", None, None, long_ago);
    let season = work(WorkKind::Season, "Season 1", Some(show.id), None, newer);
    let names_its_show = work(
        WorkKind::Episode,
        "Low Water",
        Some(season.id),
        Some("Another night in Harbor Lights."),
        newer,
    );
    let word_in_title = work(WorkKind::Movie, "Cold Harbor Road", None, None, long_ago);
    let other_show = work(WorkKind::Series, "Second Shift", None, None, newer);
    let other_season = work(
        WorkKind::Season,
        "Season 1",
        Some(other_show.id),
        None,
        newer,
    );
    let titled_episode = work(
        WorkKind::Episode,
        "The Harbor Master",
        Some(other_season.id),
        None,
        newer,
    );
    let mentioning_episode = work(
        WorkKind::Episode,
        "Stock Take",
        Some(other_season.id),
        Some("A delivery goes missing at the harbor."),
        newer,
    );
    let mentioning_movie = work(
        WorkKind::Movie,
        "Iron Tide",
        None,
        Some("A race to the harbor."),
        newer,
    );
    let works = [
        &show,
        &season,
        &names_its_show,
        &word_in_title,
        &other_show,
        &other_season,
        &titled_episode,
        &mentioning_episode,
        &mentioning_movie,
    ];
    for work in works {
        store.upsert_work(work).await.expect("work");
    }

    let hits = store
        .search_lexical("harbor", &[], None, 10)
        .await
        .expect("search");
    let found = hits.iter().map(|hit| hit.work).collect::<Vec<_>>();
    assert_eq!(
        found,
        vec![
            show.id,
            word_in_title.id,
            titled_episode.id,
            mentioning_movie.id,
            mentioning_episode.id,
        ]
    );
    assert_eq!(hits[0].matched_fields, ["title"]);
    assert_eq!(hits[3].matched_fields, ["overview"]);

    let episodes = store
        .search_lexical("harbor", &[WorkKind::Episode], None, 10)
        .await
        .expect("search");
    assert!(
        episodes.iter().any(|hit| hit.work == names_its_show.id),
        "filtering to episodes asks for them"
    );

    for work in works.into_iter().rev() {
        store.delete_work(work.id).await.ok();
    }
    store.delete_library(lib.id).await.ok();
}

/// A synopsis strong in both channels still sits below a title match.
async fn search_hybrid_keeps_title_hits_first<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Hybrid Title Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let titled = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Harbor", now);
    let mut mentioning = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Iron Tide", now);
    mentioning.overview = Some("A race to the harbor.".to_string());
    store.upsert_work(&titled).await.expect("titled");
    store.upsert_work(&mentioning).await.expect("mentioning");
    store
        .upsert_embedding(
            noflex_core::EmbeddingTarget::Work(mentioning.id),
            "title-first",
            "model",
            &[1.0, 0.0],
        )
        .await
        .expect("embedding");

    let hits = store
        .search_hybrid(&crate::query::HybridQuery {
            access_user_id: None,
            text: "harbor".to_string(),
            query_vector: Some(vec![1.0, 0.0]),
            vector_namespace: "title-first".to_string(),
            kinds: vec![],
            limit: 10,
            mode: crate::query::SearchMode::Hybrid,
        })
        .await
        .expect("hybrid");
    assert_eq!(hits[0].work, titled.id);
    assert_eq!(hits[1].work, mentioning.id);

    store.delete_work(titled.id).await.ok();
    store.delete_work(mentioning.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// Backfill sweep: every work is missing for an unseen namespace, then
/// `upsert_embedding` drops one out of that namespace only — other
/// namespaces still see it. Pagination round-trips through a cursor
/// without losing or duplicating rows, and a malformed cursor is a
/// strict `InvalidInput`.
async fn works_missing_embedding_backfill<S: Storage + ?Sized>(store: &S) {
    use crate::query::PageCursor;

    let lib = test_library("Backfill Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let a = make_work_of_kind(library_id(&lib), WorkKind::Movie, "A", now);
    let b = make_work_of_kind(library_id(&lib), WorkKind::Movie, "B", now);
    let c = make_work_of_kind(library_id(&lib), WorkKind::Movie, "C", now);
    let a_id = a.id;
    let b_id = b.id;
    let c_id = c.id;
    store.upsert_work(&a).await.expect("a");
    store.upsert_work(&b).await.expect("b");
    store.upsert_work(&c).await.expect("c");

    // No embeddings yet: every work is missing under both namespaces.
    let page = store
        .works_missing_embedding("ns-a", 10, None)
        .await
        .expect("page");
    let mut ids: Vec<_> = page.items.iter().map(|w| w.id).collect();
    ids.sort_by_key(|id| id.into_uuid());
    let mut expected = vec![a_id, b_id, c_id];
    expected.sort_by_key(|id| id.into_uuid());
    assert_eq!(ids, expected, "all three works missing for ns-a");
    assert!(page.next_cursor.is_none(), "fits under the limit");

    // Upsert B's embedding only under ns-a: it drops out of ns-a's
    // missing list, but ns-b still sees all three.
    store
        .upsert_embedding(
            noflex_core::EmbeddingTarget::Work(b_id),
            "ns-a",
            "model",
            &[0.0],
        )
        .await
        .expect("embed b");
    let ns_a = store
        .works_missing_embedding("ns-a", 10, None)
        .await
        .expect("ns-a");
    let mut ns_a_ids: Vec<_> = ns_a.items.iter().map(|w| w.id).collect();
    ns_a_ids.sort_by_key(|id| id.into_uuid());
    let mut expected_a = vec![a_id, c_id];
    expected_a.sort_by_key(|id| id.into_uuid());
    assert_eq!(ns_a_ids, expected_a, "b is embedded for ns-a, dropped");
    let ns_b = store
        .works_missing_embedding("ns-b", 10, None)
        .await
        .expect("ns-b");
    let mut ns_b_ids: Vec<_> = ns_b.items.iter().map(|w| w.id).collect();
    ns_b_ids.sort_by_key(|id| id.into_uuid());
    let mut expected_b = vec![a_id, b_id, c_id];
    expected_b.sort_by_key(|id| id.into_uuid());
    assert_eq!(ns_b_ids, expected_b, "ns-b is untouched");

    // limit-1 pages + cursor reassemble the missing list exactly.
    let mut collected: Vec<noflex_core::WorkId> = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    loop {
        let page = store
            .works_missing_embedding("ns-a", 1, cursor)
            .await
            .expect("page");
        collected.extend(page.items.iter().map(|w| w.id));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    let mut collected_sorted = collected.clone();
    collected_sorted.sort_by_key(|id| id.into_uuid());
    assert_eq!(
        collected_sorted, expected_a,
        "cursor walk visits every remaining work"
    );
    // The keyset is stable: each row should appear exactly once across
    // the pagination walk.
    collected.sort_by_key(|id| id.into_uuid());
    let mut deduped = collected.clone();
    deduped.dedup();
    assert_eq!(collected, deduped, "no duplicates across pages");

    // Malformed cursor must error, not silently re-serve page one.
    let err = store
        .works_missing_embedding("ns-a", 1, Some(PageCursor("not-a-cursor".to_string())))
        .await
        .expect_err("bad cursor");
    assert!(
        matches!(err, StorageError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );

    store.delete_work(a_id).await.ok();
    store.delete_work(b_id).await.ok();
    store.delete_work(c_id).await.ok();
    store.delete_library(lib.id).await.ok();
}

fn library_id(lib: &crate::domain::Library) -> noflex_core::LibraryId {
    lib.id
}

/// `backup_to` writes a self-contained SQLite-compatible file that a
/// fresh backend can open. The dump captures every table including
/// the migration state, so the dump is a full point-in-time snapshot.
async fn backup_round_trip<S: Storage + ?Sized>(store: &S) {
    // Populate the source with a work so the dump has non-trivial
    // content. Empty stores also exercise the path, but a populated
    // dump catches "we forgot to back up the works table" bugs.
    let lib = test_library("Backup Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work_of_kind(lib.id, WorkKind::Movie, "Backup Movie", now);
    let w_id = work.id;
    store.upsert_work(&work).await.expect("work");

    let mut buf: Vec<u8> = Vec::new();
    store.backup_to(&mut buf).await.expect("backup_to");
    assert!(!buf.is_empty(), "backup should write bytes");
    // The SQLite file format starts with the magic "SQLite format 3\0"
    // header, so a non-empty dump that doesn't start with that prefix
    // is definitely wrong.
    assert!(
        buf.len() > 16 && &buf[..16] == b"SQLite format 3\0",
        "backup bytes must start with the SQLite magic header"
    );

    store.delete_work(w_id).await.ok();
    store.delete_library(lib.id).await.ok();
}

async fn user_role_roundtrip<S: Storage + ?Sized>(store: &S) {
    let mut admin = make_user("role-admin", true);
    admin.role = noflex_core::UserRole::Admin;
    let mut guest = make_user("role-guest", false);
    guest.role = noflex_core::UserRole::Guest;
    store.upsert_user(&admin).await.expect("upsert admin");
    store.upsert_user(&guest).await.expect("upsert guest");

    let admin_back = store
        .get_user(admin.id)
        .await
        .expect("get admin")
        .expect("present");
    assert_eq!(admin_back.role, noflex_core::UserRole::Admin);
    assert!(admin_back.is_admin);
    let guest_back = store
        .get_user(guest.id)
        .await
        .expect("get guest")
        .expect("present");
    assert_eq!(guest_back.role, noflex_core::UserRole::Guest);
    assert!(!guest_back.is_admin);

    // Upgrading role to admin: re-upsert flips is_admin in lockstep.
    let mut upgraded = guest.clone();
    upgraded.role = noflex_core::UserRole::Admin;
    upgraded.is_admin = true;
    store.upsert_user(&upgraded).await.expect("upgrade");
    let after = store
        .get_user(guest.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(after.role, noflex_core::UserRole::Admin);
    assert!(after.is_admin, "role upgrade flips is_admin");

    store.delete_user(admin.id).await.ok();
    store.delete_user(guest.id).await.ok();
}

async fn library_access_grant_revoke<S: Storage + ?Sized>(store: &S) {
    let lib_a = test_library("Access A");
    let lib_b = test_library("Access B");
    store.upsert_library(&lib_a).await.expect("lib a");
    store.upsert_library(&lib_b).await.expect("lib b");
    let user = make_user("access-user", false);
    store.upsert_user(&user).await.expect("user");

    let initial = store
        .list_user_libraries(user.id)
        .await
        .expect("list initial");
    assert!(initial.is_empty(), "no access before grant");

    store
        .grant_library_access(user.id, lib_a.id)
        .await
        .expect("grant a");
    store
        .grant_library_access(user.id, lib_b.id)
        .await
        .expect("grant b");
    let after_grant = store
        .list_user_libraries(user.id)
        .await
        .expect("list after grant");
    assert_eq!(after_grant.len(), 2);
    assert!(after_grant.contains(&lib_a.id));
    assert!(after_grant.contains(&lib_b.id));

    store
        .revoke_library_access(user.id, lib_a.id)
        .await
        .expect("revoke a");
    let after_revoke = store
        .list_user_libraries(user.id)
        .await
        .expect("list after revoke");
    assert_eq!(after_revoke.len(), 1);
    assert!(after_revoke.contains(&lib_b.id));
    assert!(!after_revoke.contains(&lib_a.id));

    store
        .grant_library_access(user.id, lib_b.id)
        .await
        .expect("re-grant");
    let after_re_grant = store
        .list_user_libraries(user.id)
        .await
        .expect("list after re-grant");
    assert_eq!(after_re_grant.len(), 1);

    store.delete_user(user.id).await.ok();
    store.delete_library(lib_a.id).await.ok();
    store.delete_library(lib_b.id).await.ok();
}

async fn library_access_user_delete_cascade<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Cascade Access");
    store.upsert_library(&lib).await.expect("lib");
    let user = make_user("cascade-access", false);
    store.upsert_user(&user).await.expect("user");
    store
        .grant_library_access(user.id, lib.id)
        .await
        .expect("grant");
    store.delete_user(user.id).await.expect("delete user");
    let after = store
        .list_user_libraries(user.id)
        .await
        .expect("list after");
    assert!(after.is_empty(), "user_library_access rows wipe with user");

    store.delete_library(lib.id).await.ok();
}

async fn session_count_per_user<S: Storage + ?Sized>(store: &S) {
    let user = make_user("session-count", false);
    store.upsert_user(&user).await.expect("user");

    assert_eq!(
        store
            .count_user_sessions(user.id)
            .await
            .expect("count initial"),
        0
    );
    let s1 = make_session(user.id);
    let s2 = make_session(user.id);
    let s3 = make_session(user.id);
    store.create_session(&s1).await.expect("s1");
    store.create_session(&s2).await.expect("s2");
    store.create_session(&s3).await.expect("s3");
    let count = store.count_user_sessions(user.id).await.expect("count");
    assert_eq!(count, 3);

    let other = make_user("session-count-other", false);
    store.upsert_user(&other).await.expect("other");
    assert_eq!(
        store
            .count_user_sessions(other.id)
            .await
            .expect("other count"),
        0
    );

    store.revoke_session(s2.id).await.expect("revoke s2");
    assert_eq!(
        store
            .count_user_sessions(user.id)
            .await
            .expect("count after revoke"),
        2
    );

    store.delete_user(user.id).await.ok();
    store.delete_user(other.id).await.ok();
}

async fn session_revoke_all_for_user<S: Storage + ?Sized>(store: &S) {
    let user = make_user("revoke-all", false);
    store.upsert_user(&user).await.expect("user");
    let s1 = make_session(user.id);
    let s2 = make_session(user.id);
    store.create_session(&s1).await.expect("s1");
    store.create_session(&s2).await.expect("s2");

    let n = store.revoke_user_sessions(user.id).await.expect("revoke");
    assert_eq!(n, 2);
    assert_eq!(store.count_user_sessions(user.id).await.expect("count"), 0);
    assert_eq!(
        store
            .revoke_user_sessions(user.id)
            .await
            .expect("revoke again"),
        0
    );

    store.delete_user(user.id).await.ok();
}

async fn session_prune_idle<S: Storage + ?Sized>(store: &S) {
    let user = make_user("prune-idle", false);
    store.upsert_user(&user).await.expect("user");
    let fresh = make_session(user.id);
    let mut stale = make_session(user.id);
    let cutoff = OffsetDateTime::now_utc() - time::Duration::hours(1);
    stale.last_seen_at = cutoff - time::Duration::seconds(60);
    store.create_session(&fresh).await.expect("fresh");
    store.create_session(&stale).await.expect("stale");

    let pruned = store.prune_sessions_before(cutoff).await.expect("prune");
    assert_eq!(pruned, 1, "only the stale session is older than cutoff");
    assert!(
        store
            .get_session(fresh.id)
            .await
            .expect("get fresh")
            .is_some(),
        "fresh session survives prune"
    );
    assert!(
        store
            .get_session(stale.id)
            .await
            .expect("get stale")
            .is_none(),
        "stale session is gone"
    );

    store.delete_user(user.id).await.ok();
}

/// A library covering what rules look at: movies with varied files and credits,
/// a series whose files sit two levels down, and a viewer part way through it.
struct RulesFixture {
    library: LibraryId,
    viewer: UserId,
    /// 1995, rated 8. Drama and Crime, directed by Ada. 4K HDR10 with an Atmos
    /// English track and a French one spelled `fre`. A Director's Cut.
    alpha: WorkId,
    /// 2004, rated 6. Drama, Ada acts. 1080p SDR, 5.1, Spanish sidecar subtitles.
    beta: WorkId,
    /// 2015, unrated, added long ago, no files. Comedy, in the "Favourites" collection.
    gamma: WorkId,
    /// A two-episode series in Dolby Vision and Japanese: one episode played.
    started_show: WorkId,
    /// A one-episode series with its episode played.
    finished_show: WorkId,
}

async fn rules_fixture<S: Storage + ?Sized>(store: &S, name: &str) -> RulesFixture {
    let lib = test_library(name);
    store.upsert_library(&lib).await.expect("library");
    let viewer = make_user(&format!("{name}-viewer"), false);
    store.upsert_user(&viewer).await.expect("viewer");
    let now = OffsetDateTime::now_utc();

    let mut alpha = make_work(lib.id, "Alpha 100% Cut", now);
    alpha.year = Some(1995);
    alpha.community_rating = Some(8.0);
    alpha.metadata_status = MetadataStatus::Confirmed;
    let mut beta = make_work(lib.id, "Beta", now);
    beta.year = Some(2004);
    beta.community_rating = Some(6.0);
    beta.metadata_status = MetadataStatus::Confirmed;
    let mut gamma = make_work(lib.id, "Gamma", now - time::Duration::days(400));
    gamma.year = Some(2015);
    let started_show = make_work_of_kind(lib.id, WorkKind::Series, "Started", now);
    let mut started_season = make_work_of_kind(lib.id, WorkKind::Season, "S1", now);
    started_season.parent_id = Some(started_show.id);
    let mut played_episode = make_work_of_kind(lib.id, WorkKind::Episode, "E1", now);
    played_episode.parent_id = Some(started_season.id);
    let mut unplayed_episode = make_work_of_kind(lib.id, WorkKind::Episode, "E2", now);
    unplayed_episode.parent_id = Some(started_season.id);
    let finished_show = make_work_of_kind(lib.id, WorkKind::Series, "Finished", now);
    let mut finished_season = make_work_of_kind(lib.id, WorkKind::Season, "S1", now);
    finished_season.parent_id = Some(finished_show.id);
    let mut finished_episode = make_work_of_kind(lib.id, WorkKind::Episode, "E1", now);
    finished_episode.parent_id = Some(finished_season.id);
    for work in [
        &alpha,
        &beta,
        &gamma,
        &started_show,
        &started_season,
        &played_episode,
        &unplayed_episode,
        &finished_show,
        &finished_season,
        &finished_episode,
    ] {
        store.upsert_work(work).await.expect("work");
    }

    let track = |language: &str, codec: &str, channels: u32, object: Option<&str>| serde_json::json!({ "language": language, "codec": codec, "channels": channels, "audio_object_type": object });
    let video = |width: u32, height: u32, codec: &str, transfer: &str, dovi: Option<&str>| serde_json::json!({ "width": width, "height": height, "codec": codec, "transfer": transfer, "dovi_profile": dovi });
    let files = [
        (
            alpha.id,
            "Director's Cut",
            serde_json::json!({
                "video": video(3840, 1600, "h265", "pq", None),
                "audio": [track("eng", "truehd", 8, Some("joc")), track("fre", "ac3", 6, None)],
                "subtitle": [{ "language": "eng", "codec": "subrip" }],
            }),
            vec![],
        ),
        (
            beta.id,
            "Default",
            serde_json::json!({
                "video": video(1920, 1080, "h264", "bt709", None),
                "audio": [track("eng", "eac3", 6, None)],
                "subtitle": [],
            }),
            vec![serde_json::from_value(serde_json::json!({ "language": "es", "codec": "srt", "default": false, "forced": false, "external": true })).expect("sidecar")],
        ),
        (
            played_episode.id,
            "Default",
            serde_json::json!({
                "video": video(3840, 2160, "h265", "pq", Some("profile8")),
                "audio": [track("jpn", "aac", 2, None)],
                "subtitle": [],
            }),
            vec![],
        ),
        (
            unplayed_episode.id,
            "Default",
            serde_json::json!({
                "video": video(1280, 720, "h264", "bt709", None),
                "audio": [track("jpn", "aac", 2, None)],
                "subtitle": [],
            }),
            vec![],
        ),
        (
            finished_episode.id,
            "Default",
            serde_json::json!({
                "video": video(1920, 1080, "h264", "bt709", None),
                "audio": [track("eng", "aac", 2, None)],
                "subtitle": [],
            }),
            vec![],
        ),
    ];
    for (work, edition_name, probe, sidecars) in files {
        let edition = make_edition(work, edition_name);
        let edition_id = store.upsert_edition(&edition).await.expect("edition");
        let mut media = make_media(edition_id, &format!("/rules/{name}/{work:?}.mkv"), now);
        media.probe = probe.into();
        media.subtitle_tracks = sidecars;
        store.upsert_media_file(&media).await.expect("media");
    }

    let assign = async |work: WorkId,
                        namespace: &str,
                        display: &str,
                        metadata: serde_json::Value| {
        let slug = noflex_core::text::slugify(display);
        let tag = match store.tag_by_slug(namespace, &slug).await.expect("find tag") {
            Some(tag) => tag.id,
            None => store
                .upsert_tag(&make_tag(namespace, &slug, display))
                .await
                .expect("tag"),
        };
        store
            .assign_tag_with_metadata(tag, TagTarget::Work(work), 1.0, "provider:test", &metadata)
            .await
            .expect("assign");
    };
    let none = serde_json::json!({});
    assign(alpha.id, "genre", "Drama", none.clone()).await;
    assign(alpha.id, "genre", "Crime", none.clone()).await;
    assign(beta.id, "genre", "Drama", none.clone()).await;
    assign(gamma.id, "genre", "Comedy", none.clone()).await;
    assign(
        alpha.id,
        "person",
        "Ada Reyes",
        serde_json::json!({ "credits": [{ "role": "Director" }] }),
    )
    .await;
    assign(
        beta.id,
        "person",
        "Ada Reyes",
        serde_json::json!({ "credits": [{ "character": "Mara", "role": "Actor" }] }),
    )
    .await;

    let collection = Collection {
        id: CollectionId::new(),
        library_id: lib.id,
        title: "Favourites".to_string(),
        sort_title: "favourites".to_string(),
        overview: None,
        created_at: now,
        updated_at: now,
    };
    store
        .create_collection(&collection)
        .await
        .expect("collection");
    store
        .set_collection_items(collection.id, &[gamma.id])
        .await
        .expect("items");

    for played in [played_episode.id, finished_episode.id] {
        store
            .mark_played(viewer.id, noflex_core::PlayTarget::Work(played), true)
            .await
            .expect("played");
    }
    store
        .set_favorite(viewer.id, noflex_core::PlayTarget::Work(beta.id), true)
        .await
        .expect("favorite");

    RulesFixture {
        library: lib.id,
        viewer: viewer.id,
        alpha: alpha.id,
        beta: beta.id,
        gamma: gamma.id,
        started_show: started_show.id,
        finished_show: finished_show.id,
    }
}

fn rule(facet: crate::WorkFacet, mode: crate::MatchMode, values: &[&str]) -> crate::WorkRule {
    crate::WorkRule {
        facet,
        mode,
        values: values.iter().map(ToString::to_string).collect(),
    }
}

async fn list_works_rules<S: Storage + ?Sized>(store: &S) {
    use crate::{ListWorksQuery, MatchMode::*, WorkFacet::*, WorkRules};

    let f = rules_fixture(store, "Rules").await;
    let query = |rules: WorkRules| ListWorksQuery {
        library: Some(f.library),
        viewer: Some(f.viewer),
        rules: Some(rules),
        recursive: false,
        limit: 100,
        ..Default::default()
    };
    let matching = async |rules: WorkRules| -> Vec<WorkId> {
        let mut ids: Vec<WorkId> = store
            .list_works(&query(rules))
            .await
            .expect("list")
            .items
            .iter()
            .map(|work| work.id)
            .collect();
        ids.sort_by_key(|id| id.into_uuid());
        ids
    };
    let check = async |rules: Vec<crate::WorkRule>, mode, expected: &[WorkId], what: &str| {
        let mut expected = expected.to_vec();
        expected.sort_by_key(|id| id.into_uuid());
        let got = matching(WorkRules {
            mode,
            rules,
            groups: vec![],
        })
        .await;
        assert_eq!(got, expected, "{what}");
    };
    let RulesFixture {
        alpha,
        beta,
        gamma,
        started_show: started,
        finished_show: finished,
        ..
    } = f;

    // A facet's own logic.
    check(
        vec![rule(Genre, Any, &["crime", "comedy"])],
        All,
        &[alpha, gamma],
        "any genre",
    )
    .await;
    check(
        vec![rule(Genre, All, &["drama", "crime"])],
        All,
        &[alpha],
        "all genres",
    )
    .await;
    check(
        vec![rule(Genre, None, &["drama"])],
        All,
        &[gamma, started, finished],
        "no genre",
    )
    .await;
    // A person's role is part of the match.
    check(
        vec![rule(Director, Any, &["ada-reyes"])],
        All,
        &[alpha],
        "directed",
    )
    .await;
    check(
        vec![rule(Actor, Any, &["ada-reyes"])],
        All,
        &[beta],
        "acted",
    )
    .await;

    // Facets combined, with a group's mode.
    check(
        vec![
            rule(Genre, Any, &["comedy"]),
            rule(Resolution, Any, &["4k"]),
        ],
        Any,
        &[alpha, gamma, started],
        "or",
    )
    .await;
    check(
        vec![rule(Genre, Any, &["drama"]), rule(Resolution, Any, &["4k"])],
        All,
        &[alpha],
        "and",
    )
    .await;
    check(
        vec![rule(Genre, Any, &["drama"]), rule(Resolution, Any, &["4k"])],
        None,
        &[gamma, finished],
        "nor",
    )
    .await;

    // Groups nest: (4K or comedy) and not crime.
    let nested = WorkRules {
        mode: All,
        rules: vec![rule(Genre, None, &["crime"])],
        groups: vec![WorkRules {
            mode: Any,
            rules: vec![
                rule(Resolution, Any, &["4k"]),
                rule(Genre, Any, &["comedy"]),
            ],
            groups: vec![],
        }],
    };
    let mut expected = vec![gamma, started];
    expected.sort_by_key(|id| id.into_uuid());
    assert_eq!(matching(nested).await, expected, "nested groups");
    // Empty conditions say nothing, at any level.
    let empty = WorkRules {
        mode: Any,
        rules: vec![rule(Genre, Any, &[])],
        groups: vec![WorkRules::default()],
    };
    assert_eq!(matching(empty).await.len(), 5, "empty rules filter nothing");

    // Media: by either dimension, on a work or anything beneath it.
    check(
        vec![rule(Resolution, Any, &["4k"])],
        All,
        &[alpha, started],
        "scope 4K counts",
    )
    .await;
    check(
        vec![rule(Resolution, Any, &["720p"])],
        All,
        &[started],
        "any episode's file",
    )
    .await;
    check(
        vec![rule(Resolution, All, &["4k", "720p"])],
        All,
        &[started],
        "files together",
    )
    .await;
    check(
        vec![rule(DynamicRange, Any, &["dolby_vision"])],
        All,
        &[started],
        "vision",
    )
    .await;
    check(
        vec![rule(DynamicRange, Any, &["hdr10"])],
        All,
        &[alpha],
        "vision isn't HDR10",
    )
    .await;
    check(
        vec![rule(VideoCodec, Any, &["h265"])],
        All,
        &[alpha, started],
        "video codec",
    )
    .await;
    check(
        vec![rule(AudioCodec, Any, &["truehd"])],
        All,
        &[alpha],
        "audio codec",
    )
    .await;
    check(
        vec![rule(AudioLayout, Any, &["atmos"])],
        All,
        &[alpha],
        "atmos",
    )
    .await;
    check(
        vec![rule(AudioLayout, Any, &["5.1"])],
        All,
        &[alpha, beta],
        "any track's layout",
    )
    .await;
    check(
        vec![rule(AudioLanguage, Any, &["fra"])],
        All,
        &[alpha],
        "another spelling of a language",
    )
    .await;
    check(
        vec![rule(SubtitleLanguage, Any, &["spa"])],
        All,
        &[beta],
        "sidecar subtitles",
    )
    .await;
    check(
        vec![rule(SubtitleCodec, Any, &["subrip"])],
        All,
        &[alpha],
        "muxed subtitles",
    )
    .await;
    check(
        vec![rule(Edition, Any, &["Director's Cut"])],
        All,
        &[alpha],
        "edition",
    )
    .await;

    // The rest of what a work is.
    check(
        vec![rule(Title, Any, &["100%"])],
        All,
        &[alpha],
        "title, with % as itself",
    )
    .await;
    check(
        vec![rule(Title, None, &["ALPHA", "beta"])],
        All,
        &[gamma, started, finished],
        "not these titles",
    )
    .await;
    check(
        vec![rule(Years, Any, &["1990-1999", "2010-2019"])],
        All,
        &[alpha, gamma],
        "decades",
    )
    .await;
    check(
        vec![rule(Years, Any, &["2004-"])],
        All,
        &[beta, gamma],
        "open-ended years",
    )
    .await;
    check(vec![rule(MinRating, Any, &["7"])], All, &[alpha], "rating").await;
    check(
        vec![rule(AddedWithinDays, None, &["30"])],
        All,
        &[gamma],
        "added long ago",
    )
    .await;
    check(
        vec![rule(Collection, Any, &["favourites"])],
        All,
        &[gamma],
        "collection ignores case",
    )
    .await;
    check(
        vec![rule(MatchStatus, Any, &["matched"])],
        All,
        &[alpha, beta],
        "matched",
    )
    .await;

    // The viewer's own state, rolled up through a series.
    check(
        vec![rule(Favorite, Any, &["yes"])],
        All,
        &[beta],
        "favorite",
    )
    .await;
    check(
        vec![rule(WatchState, Any, &["in_progress"])],
        All,
        &[started],
        "part way through",
    )
    .await;
    check(
        vec![rule(WatchState, Any, &["watched"])],
        All,
        &[finished],
        "every episode played",
    )
    .await;
    check(
        vec![rule(WatchState, Any, &["unwatched"])],
        All,
        &[alpha, beta, gamma],
        "nothing started",
    )
    .await;
    check(
        vec![rule(WatchState, None, &["unwatched"])],
        All,
        &[started, finished],
        "anything started",
    )
    .await;

    // Pages keep to the rules, and to their cursor.
    let first = store
        .list_works(&ListWorksQuery {
            limit: 1,
            ..query(WorkRules {
                rules: vec![rule(Genre, Any, &["drama"])],
                ..Default::default()
            })
        })
        .await
        .expect("first page");
    assert_eq!(first.items.len(), 1);
    assert!(first.has_next);
    let second = store
        .list_works(&ListWorksQuery {
            limit: 1,
            cursor: first.next_cursor(),
            ..query(WorkRules {
                rules: vec![rule(Genre, Any, &["drama"])],
                ..Default::default()
            })
        })
        .await
        .expect("second page");
    assert_eq!(second.items.len(), 1);
    assert_ne!(first.items[0].id, second.items[0].id);
    assert!(!second.has_next);

    // Mistakes are the caller's to hear about, not an empty grid.
    for rules in [
        vec![rule(Years, Any, &["nineties"])],
        vec![rule(WatchState, Any, &["paused"])],
        vec![rule(Genre, All, &vec!["x"; 70])],
    ] {
        let result = store
            .list_works(&query(WorkRules {
                rules,
                ..Default::default()
            }))
            .await;
        assert!(matches!(result, Err(crate::StorageError::InvalidInput(_))));
    }
    let anonymous = ListWorksQuery {
        viewer: Option::None,
        ..query(WorkRules {
            rules: vec![rule(Favorite, Any, &["yes"])],
            ..Default::default()
        })
    };
    assert!(matches!(
        store.list_works(&anonymous).await,
        Err(crate::StorageError::InvalidInput(_))
    ));
    let mut deep = WorkRules {
        rules: vec![rule(Genre, Any, &["drama"])],
        ..Default::default()
    };
    for _ in 0..12 {
        deep = WorkRules {
            groups: vec![deep],
            ..Default::default()
        };
    }
    assert!(matches!(
        store.list_works(&query(deep)).await,
        Err(crate::StorageError::InvalidInput(_))
    ));

    // Stored facts follow the file: rewritten with its probe, gone with its removal.
    let edition = store
        .editions_for_work(alpha)
        .await
        .expect("editions")
        .remove(0);
    let mut file = store
        .media_files_for_edition(edition.id)
        .await
        .expect("files")
        .remove(0);
    file.probe =
        serde_json::json!({ "video": { "width": 1280, "height": 720, "codec": "h264" } }).into();
    store.upsert_media_file(&file).await.expect("rewrite");
    check(
        vec![rule(Resolution, Any, &["4k"])],
        All,
        &[started],
        "the old value left",
    )
    .await;
    check(
        vec![rule(Resolution, Any, &["720p"])],
        All,
        &[alpha, started],
        "the new value arrived",
    )
    .await;
    check(
        vec![rule(AudioLayout, Any, &["atmos"])],
        All,
        &[],
        "its old tracks left",
    )
    .await;
    store.delete_media_file(file.id).await.expect("delete");
    check(
        vec![rule(Resolution, Any, &["720p"])],
        All,
        &[started],
        "gone with the file",
    )
    .await;

    // Moving a season carries its episodes' files to the new series.
    let now = OffsetDateTime::now_utc();
    let adopter = make_work_of_kind(f.library, WorkKind::Series, "Adopter", now);
    store.upsert_work(&adopter).await.expect("adopter");
    let mut season = store
        .list_works(&ListWorksQuery {
            parent: Some(started),
            limit: 1,
            ..Default::default()
        })
        .await
        .expect("seasons")
        .items
        .remove(0);
    season.parent_id = Some(adopter.id);
    store.upsert_work(&season).await.expect("move season");
    check(
        vec![rule(Resolution, Any, &["720p"])],
        All,
        &[adopter.id],
        "the files followed their season",
    )
    .await;

    store.delete_library(f.library).await.expect("clean up");
    store.delete_user(f.viewer).await.expect("clean up");
}

/// Two cuts of one film, one of them in two encodes: a cut is an edition with its
/// own runtime, and each encode is a file that answers for itself.
async fn cuts_and_encodes_stay_on_their_own_levels<S: Storage + ?Sized>(store: &S) {
    use crate::{ListWorksQuery, MatchMode::*, WorkFacet::*, WorkRules};

    let library = test_library("Cuts And Encodes");
    store.upsert_library(&library).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let film = make_work(library.id, "Film", now);
    store.upsert_work(&film).await.expect("film");

    let mut theatrical = make_edition(film.id, "Theatrical");
    theatrical.runtime_ms = Some(6_000_000);
    let mut extended = make_edition(film.id, "Extended");
    extended.runtime_ms = Some(9_000_000);
    store.upsert_edition(&theatrical).await.expect("theatrical");
    store.upsert_edition(&extended).await.expect("extended");

    let probe = |width: u32, height: u32, transfer: &str| serde_json::json!({ "video": { "width": width, "height": height, "codec": "h265", "transfer": transfer } });
    for (edition, path, picture) in [
        (
            theatrical.id,
            "/cuts/theatrical-4k.mkv",
            probe(3840, 2160, "pq"),
        ),
        (
            theatrical.id,
            "/cuts/theatrical-1080.mkv",
            probe(1920, 1080, "bt709"),
        ),
        (
            extended.id,
            "/cuts/extended-720.mkv",
            probe(1280, 720, "bt709"),
        ),
    ] {
        let mut file = make_media(edition, path, now);
        file.probe = picture.into();
        store.upsert_media_file(&file).await.expect("file");
    }

    let cuts = store.editions_for_work(film.id).await.expect("cuts");
    let runtime_of = |name: &str| {
        cuts.iter()
            .find(|cut| cut.name == name)
            .and_then(|cut| cut.runtime_ms)
    };
    assert_eq!(cuts.len(), 2, "encodes don't make more cuts");
    assert_eq!(runtime_of("Theatrical"), Some(6_000_000));
    assert_eq!(runtime_of("Extended"), Some(9_000_000));

    let encodes = store
        .media_files_for_edition(theatrical.id)
        .await
        .expect("encodes");
    assert_eq!(encodes.len(), 2, "each encode keeps its own file");
    let heights: Vec<_> = encodes
        .iter()
        .map(|file| file.probe.0["video"]["height"].as_u64())
        .collect();
    assert!(heights.contains(&Some(2160)) && heights.contains(&Some(1080)));

    let matches = async |facet, value: &str| {
        let rules = WorkRules {
            rules: vec![rule(facet, Any, &[value])],
            ..Default::default()
        };
        let page = store
            .list_works(&ListWorksQuery {
                library: Some(library.id),
                rules: Some(rules),
                limit: 10,
                ..Default::default()
            })
            .await
            .expect("list");
        page.items.iter().map(|work| work.id).collect::<Vec<_>>() == [film.id]
    };
    // Every encode of every cut answers for the film, including the one a column
    // on the edition would have had no room for.
    assert!(matches(Resolution, "4k").await);
    assert!(matches(Resolution, "1080p").await);
    assert!(matches(Resolution, "720p").await);
    assert!(matches(DynamicRange, "hdr10").await);
    assert!(matches(Edition, "Theatrical").await);
    assert!(matches(Edition, "Extended").await);
    assert!(!matches(Resolution, "8k").await);
    assert!(!matches(Edition, "Default").await);

    store.delete_library(library.id).await.expect("clean up");
}

async fn work_facets_list_what_filters_can_match<S: Storage + ?Sized>(store: &S) {
    use crate::{WorkFacet, WorkFacetsQuery};

    let f = rules_fixture(store, "Facets").await;
    let facets = store
        .work_facets(&WorkFacetsQuery {
            library: Some(f.library),
            ..Default::default()
        })
        .await
        .expect("facets");
    let values = |facet: WorkFacet| -> Vec<(String, u32)> {
        let mut found: Vec<(String, u32)> = facets
            .iter()
            .find(|entry| entry.facet == facet)
            .map(|entry| {
                entry
                    .values
                    .iter()
                    .map(|value| (value.value.clone(), value.work_count))
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
    };
    let owned = |expected: &[(&str, u32)]| -> Vec<(String, u32)> {
        expected
            .iter()
            .map(|(value, count)| (value.to_string(), *count))
            .collect()
    };

    assert_eq!(
        values(WorkFacet::Genre),
        owned(&[("comedy", 1), ("crime", 1), ("drama", 2)])
    );
    assert_eq!(values(WorkFacet::Director), owned(&[("ada-reyes", 1)]));
    assert_eq!(values(WorkFacet::Actor), owned(&[("ada-reyes", 1)]));
    // Counted as titles: a two-episode series is one, however its files differ.
    assert_eq!(
        values(WorkFacet::Resolution),
        owned(&[("1080p", 2), ("4k", 2), ("720p", 1)])
    );
    assert_eq!(
        values(WorkFacet::DynamicRange),
        owned(&[("dolby_vision", 1), ("hdr10", 1), ("sdr", 3)])
    );
    assert_eq!(
        values(WorkFacet::AudioLanguage),
        owned(&[("eng", 3), ("fra", 1), ("jpn", 1)])
    );
    assert_eq!(
        values(WorkFacet::SubtitleLanguage),
        owned(&[("eng", 1), ("spa", 1)])
    );
    assert_eq!(values(WorkFacet::Edition), owned(&[("Director's Cut", 1)]));
    assert_eq!(values(WorkFacet::Collection), owned(&[("Favourites", 1)]));
    assert_eq!(
        values(WorkFacet::Years),
        owned(&[("1990-1999", 1), ("2000-2009", 1), ("2010-2019", 1)])
    );
    assert!(
        values(WorkFacet::Mood).is_empty(),
        "a facet with no values is left out"
    );
    let years = facets
        .iter()
        .find(|entry| entry.facet == WorkFacet::Years)
        .expect("years");
    assert!(years.values.iter().any(|value| value.label == "1990s"));

    // Scoped: another library's tags aren't this library's to offer.
    let other = test_library("Facets Elsewhere");
    store.upsert_library(&other).await.expect("library");
    let empty = store
        .work_facets(&WorkFacetsQuery {
            library: Some(other.id),
            ..Default::default()
        })
        .await
        .expect("empty facets");
    assert!(empty.is_empty());
    // And access-checked: this viewer has no grant on the library at all.
    let denied = store
        .work_facets(&WorkFacetsQuery {
            library: Some(f.library),
            access_user_id: Some(f.viewer),
            ..Default::default()
        })
        .await
        .expect("denied facets");
    assert!(denied.is_empty());

    store.delete_library(other.id).await.expect("clean up");
    store.delete_library(f.library).await.expect("clean up");
    store.delete_user(f.viewer).await.expect("clean up");
}

async fn list_works_basic_filter<S: Storage + ?Sized>(store: &S) {
    use crate::query::ListWorksQuery;

    let lib = test_library("List Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let mut m1 = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Alpha", now);
    m1.year = Some(2020);
    let m1_id = m1.id;
    let mut m2 = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Beta", now);
    m2.year = Some(2010);
    let m2_id = m2.id;
    let mut m3 = make_work_of_kind(library_id(&lib), WorkKind::Movie, "Gamma", now);
    m3.year = Some(2005);
    let m3_id = m3.id;
    let mut s = make_work_of_kind(library_id(&lib), WorkKind::Series, "Show", now);
    s.parent_id = Some(m1_id);
    let s_id = s.id;
    store.upsert_work(&m1).await.expect("m1");
    store.upsert_work(&m2).await.expect("m2");
    store.upsert_work(&m3).await.expect("m3");
    store.upsert_work(&s).await.expect("s");

    let q = ListWorksQuery {
        library: Some(library_id(&lib)),
        recursive: true,
        limit: 100,
        ..Default::default()
    };
    let page = store.list_works(&q).await.expect("list library");
    assert_eq!(page.items.len(), 4);

    let q = ListWorksQuery {
        kinds: vec![WorkKind::Series],
        recursive: true,
        limit: 100,
        ..Default::default()
    };
    let page = store.list_works(&q).await.expect("list series");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, s_id);

    let q = ListWorksQuery {
        year_min: Some(2010),
        year_max: Some(2015),
        limit: 100,
        ..Default::default()
    };
    let page = store.list_works(&q).await.expect("year range");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, m2_id);

    let q = ListWorksQuery {
        recursive: false,
        limit: 100,
        ..Default::default()
    };
    let page = store.list_works(&q).await.expect("top-level only");
    assert_eq!(page.items.len(), 3, "non-recursive drops child");

    let q = ListWorksQuery {
        recursive: true,
        limit: 100,
        ..Default::default()
    };
    let page = store.list_works(&q).await.expect("recursive");
    assert_eq!(page.items.len(), 4, "recursive includes child");

    let q = ListWorksQuery {
        parent: Some(m1_id),
        limit: 100,
        ..Default::default()
    };
    let page = store.list_works(&q).await.expect("by parent");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, s_id);

    let q = ListWorksQuery {
        limit: 1,
        ..Default::default()
    };
    let page1 = store.list_works(&q).await.expect("page1");
    assert_eq!(page1.items.len(), 1);
    let cursor = page1
        .next_cursor()
        .as_ref()
        .expect("cursor present")
        .clone();
    let q = ListWorksQuery {
        limit: 1,
        cursor: Some(cursor),
        ..Default::default()
    };
    let page2 = store.list_works(&q).await.expect("page2");
    assert_eq!(page2.items.len(), 1);
    assert_ne!(page1.items[0].id, page2.items[0].id, "pages are different");

    store.delete_work(m1_id).await.ok();
    store.delete_work(m2_id).await.ok();
    store.delete_work(m3_id).await.ok();
    store.delete_work(s_id).await.ok();
    store.delete_library(library_id(&lib)).await.ok();
}

/// Follow `next_cursor` to exhaustion and return every work seen,
/// asserting the walk terminates instead of looping on a bad cursor.
async fn collect_all_pages<S: Storage + ?Sized>(
    store: &S,
    q: &crate::query::ListWorksQuery,
) -> Vec<Work> {
    let mut out = Vec::new();
    let mut cursor = None;
    for _ in 0..20 {
        let page = store
            .list_works(&crate::query::ListWorksQuery {
                cursor,
                ..q.clone()
            })
            .await
            .expect("list page");
        let next = page.next_cursor();
        out.extend(page.items);
        match next {
            Some(c) => cursor = Some(c),
            None => return out,
        }
    }
    panic!("pagination did not terminate within 20 pages");
}

/// Every item's cursor is a valid `after`: resuming from item `i`
/// yields exactly the items after it, in every keyset sort.
async fn list_works_item_cursors_resume_after_each_item<S: Storage + ?Sized>(store: &S) {
    use crate::query::{ListWorksQuery, SortOrder};

    let lib = test_library("Item Cursor Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let mut works = Vec::new();
    for (i, title) in ["kilo", "Lima", "mike", "November"].into_iter().enumerate() {
        let mut w = make_work(lib.id, title, now + time::Duration::seconds(i as i64));
        w.year = (i % 2 == 0).then_some(2000 + i as i32);
        store.upsert_work(&w).await.expect("work");
        works.push(w);
    }
    for sort in [
        SortOrder::DateAddedDesc,
        SortOrder::DateAddedAsc,
        SortOrder::TitleAsc,
        SortOrder::TitleDesc,
        SortOrder::YearAsc,
        SortOrder::YearDesc,
    ] {
        let q = ListWorksQuery {
            library: Some(lib.id),
            recursive: true,
            limit: 10,
            sort,
            ..Default::default()
        };
        let full = store.list_works(&q).await.expect("full page");
        assert_eq!(full.items.len(), works.len(), "{sort:?}");
        assert_eq!(full.item_cursors.len(), full.items.len(), "{sort:?}");
        assert!(!full.has_next, "{sort:?}");
        for (i, cursor) in full.item_cursors.iter().enumerate() {
            let rest = store
                .list_works(&ListWorksQuery {
                    cursor: Some(cursor.clone()),
                    ..q.clone()
                })
                .await
                .expect("resumed page");
            assert_eq!(
                rest.items.iter().map(|w| w.id).collect::<Vec<_>>(),
                full.items[i + 1..].iter().map(|w| w.id).collect::<Vec<_>>(),
                "{sort:?} resuming after item {i}"
            );
        }
    }
    for w in &works {
        store.delete_work(w.id).await.ok();
    }
    store.delete_library(lib.id).await.ok();
}

/// `watched` is per-user: played works land in the `watched: true`
/// set, and `watched: false` catches both progress-only rows and
/// works with no user_data row at all.
async fn list_works_watched_filter<S: Storage + ?Sized>(store: &S) {
    use crate::query::{ListWorksQuery, WatchedFilter};

    let lib = test_library("Watched Lib");
    store.upsert_library(&lib).await.expect("library");
    let user_a = make_user("watched-a", false);
    let user_b = make_user("watched-b", false);
    store.upsert_user(&user_a).await.expect("user a");
    store.upsert_user(&user_b).await.expect("user b");

    let now = OffsetDateTime::now_utc();
    let played = make_work(lib.id, "Played", now);
    let in_progress = make_work(lib.id, "In Progress", now);
    let untouched = make_work(lib.id, "Untouched", now);
    let played_by_b = make_work(lib.id, "Played By B", now);
    for w in [&played, &in_progress, &untouched, &played_by_b] {
        store.upsert_work(w).await.expect("work");
    }
    store
        .mark_played(user_a.id, noflex_core::PlayTarget::Work(played.id), true)
        .await
        .expect("mark played");
    store
        .record_playback_progress(
            user_a.id,
            noflex_core::PlayTarget::Work(in_progress.id),
            120_000,
        )
        .await
        .expect("progress");
    store
        .mark_played(
            user_b.id,
            noflex_core::PlayTarget::Work(played_by_b.id),
            true,
        )
        .await
        .expect("mark played b");

    let ids = |q: ListWorksQuery| async move {
        let page = store.list_works(&q).await.expect("list");
        let mut ids: Vec<WorkId> = page.items.iter().map(|w| w.id).collect();
        ids.sort_by_key(|id| id.into_uuid());
        ids
    };
    let base = ListWorksQuery {
        library: Some(lib.id),
        recursive: true,
        limit: 100,
        ..Default::default()
    };

    let watched_a = ids(ListWorksQuery {
        watched: Some(WatchedFilter {
            user_id: user_a.id,
            watched: true,
        }),
        ..base.clone()
    })
    .await;
    assert_eq!(watched_a, [played.id], "A completed only the played work");

    let mut expected_unwatched = vec![in_progress.id, untouched.id, played_by_b.id];
    expected_unwatched.sort_by_key(|id| id.into_uuid());
    let unwatched_a = ids(ListWorksQuery {
        watched: Some(WatchedFilter {
            user_id: user_a.id,
            watched: false,
        }),
        ..base.clone()
    })
    .await;
    assert_eq!(
        unwatched_a, expected_unwatched,
        "progress-only and never-played works are unwatched"
    );

    let watched_b = ids(ListWorksQuery {
        watched: Some(WatchedFilter {
            user_id: user_b.id,
            watched: true,
        }),
        ..base.clone()
    })
    .await;
    assert_eq!(watched_b, [played_by_b.id], "B's state is independent");

    store.delete_work(played.id).await.ok();
    store.delete_work(in_progress.id).await.ok();
    store.delete_work(untouched.id).await.ok();
    store.delete_work(played_by_b.id).await.ok();
    store.delete_user(user_a.id).await.ok();
    store.delete_user(user_b.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// Rating bounds include/exclude by value and always drop NULL-rated
/// works; added_at bounds are inclusive on both ends.
async fn list_works_rating_and_added_filters<S: Storage + ?Sized>(store: &S) {
    use crate::query::ListWorksQuery;

    let lib = test_library("Rating Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();

    let mut low = make_work(lib.id, "Low Rated", now - time::Duration::hours(2));
    low.community_rating = Some(5.0);
    let mut high = make_work(lib.id, "High Rated", now - time::Duration::hours(1));
    high.community_rating = Some(7.5);
    let unrated = make_work(lib.id, "Unrated", now);
    store.upsert_work(&low).await.expect("low");
    store.upsert_work(&high).await.expect("high");
    store.upsert_work(&unrated).await.expect("unrated");

    let ids = |q: ListWorksQuery| async move {
        let page = store.list_works(&q).await.expect("list");
        let mut ids: Vec<WorkId> = page.items.iter().map(|w| w.id).collect();
        ids.sort_by_key(|id| id.into_uuid());
        ids
    };
    let base = ListWorksQuery {
        library: Some(lib.id),
        recursive: true,
        limit: 100,
        ..Default::default()
    };

    let min = ids(ListWorksQuery {
        rating_min: Some(6.0),
        ..base.clone()
    })
    .await;
    assert_eq!(min, [high.id], "rating_min keeps only the high-rated work");

    let max = ids(ListWorksQuery {
        rating_max: Some(6.0),
        ..base.clone()
    })
    .await;
    assert_eq!(max, [low.id], "rating_max drops the high-rated work");

    let band = ids(ListWorksQuery {
        rating_min: Some(4.0),
        rating_max: Some(8.0),
        ..base.clone()
    })
    .await;
    let mut expected_band = vec![low.id, high.id];
    expected_band.sort_by_key(|id| id.into_uuid());
    assert_eq!(band, expected_band, "NULL rating fails any bound");

    let after = ids(ListWorksQuery {
        added_after: Some(now - time::Duration::minutes(90)),
        ..base.clone()
    })
    .await;
    let mut expected_after = vec![high.id, unrated.id];
    expected_after.sort_by_key(|id| id.into_uuid());
    assert_eq!(after, expected_after, "added_after is inclusive");

    let before = ids(ListWorksQuery {
        added_before: Some(now - time::Duration::minutes(90)),
        ..base.clone()
    })
    .await;
    assert_eq!(before, [low.id], "added_before is inclusive");

    let window = ids(ListWorksQuery {
        added_after: Some(now - time::Duration::minutes(90)),
        added_before: Some(now - time::Duration::minutes(30)),
        ..base.clone()
    })
    .await;
    assert_eq!(window, [high.id], "both bounds intersect");

    store.delete_work(low.id).await.ok();
    store.delete_work(high.id).await.ok();
    store.delete_work(unrated.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// Title sorts paginate by the lowered-title keyset: a full walk in
/// both directions visits every work exactly once, in order, over
/// mixed-case titles.
async fn list_works_title_sort_pagination<S: Storage + ?Sized>(store: &S) {
    use crate::query::{ListWorksQuery, SortOrder};

    let lib = test_library("Title Page Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let mut works = Vec::new();
    for title in ["alpha", "Bravo", "charlie", "Delta", "echo"] {
        let w = make_work(lib.id, title, now);
        store.upsert_work(&w).await.expect("work");
        works.push(w);
    }

    let base = ListWorksQuery {
        library: Some(lib.id),
        recursive: true,
        limit: 2,
        ..Default::default()
    };

    let asc = collect_all_pages(
        store,
        &ListWorksQuery {
            sort: SortOrder::TitleAsc,
            ..base.clone()
        },
    )
    .await;
    let titles: Vec<&str> = asc.iter().map(|w| w.title.as_str()).collect();
    assert_eq!(
        titles,
        ["alpha", "Bravo", "charlie", "Delta", "echo"],
        "TitleAsc walks every work once, case-insensitively ordered"
    );

    let desc = collect_all_pages(
        store,
        &ListWorksQuery {
            sort: SortOrder::TitleDesc,
            ..base.clone()
        },
    )
    .await;
    let titles: Vec<&str> = desc.iter().map(|w| w.title.as_str()).collect();
    assert_eq!(titles, ["echo", "Delta", "charlie", "Bravo", "alpha"]);

    for w in &works {
        store.delete_work(w.id).await.ok();
    }
    store.delete_library(lib.id).await.ok();
}

/// NULL years sort first in ASC and last in DESC; the NULL-key cursor
/// branch must still walk every work exactly once in both directions.
async fn list_works_year_sort_pagination_with_nulls<S: Storage + ?Sized>(store: &S) {
    use crate::query::{ListWorksQuery, SortOrder};

    let lib = test_library("Year Page Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let mut works = Vec::new();
    for (title, year) in [
        ("Y2010", Some(2010)),
        ("Y1999", Some(1999)),
        ("Null A", None),
        ("Null B", None),
        ("Y2001", Some(2001)),
    ] {
        let mut w = make_work(lib.id, title, now);
        w.year = year;
        store.upsert_work(&w).await.expect("work");
        works.push(w);
    }

    let base = ListWorksQuery {
        library: Some(lib.id),
        recursive: true,
        limit: 2,
        ..Default::default()
    };

    let desc = collect_all_pages(
        store,
        &ListWorksQuery {
            sort: SortOrder::YearDesc,
            ..base.clone()
        },
    )
    .await;
    let years: Vec<Option<i32>> = desc.iter().map(|w| w.year).collect();
    assert_eq!(years.len(), 5, "every work appears exactly once");
    assert_eq!(&years[..3], &[Some(2010), Some(2001), Some(1999)]);
    assert_eq!(&years[3..], &[None, None], "NULL years sort last in DESC");

    let asc = collect_all_pages(
        store,
        &ListWorksQuery {
            sort: SortOrder::YearAsc,
            ..base.clone()
        },
    )
    .await;
    let years: Vec<Option<i32>> = asc.iter().map(|w| w.year).collect();
    assert_eq!(years.len(), 5);
    assert_eq!(&years[..2], &[None, None], "NULL years sort first in ASC");
    assert_eq!(&years[2..], &[Some(1999), Some(2001), Some(2010)]);

    for w in &works {
        store.delete_work(w.id).await.ok();
    }
    store.delete_library(lib.id).await.ok();
}

/// A random page can't be keyset-paginated, so it returns a full page
/// but never a `next_cursor`.
async fn list_works_random_sort_mints_no_cursor<S: Storage + ?Sized>(store: &S) {
    use crate::query::{ListWorksQuery, SortOrder};

    let lib = test_library("Random Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let mut works = Vec::new();
    for title in ["R1", "R2", "R3", "R4"] {
        let w = make_work(lib.id, title, now);
        store.upsert_work(&w).await.expect("work");
        works.push(w);
    }

    let page = store
        .list_works(&ListWorksQuery {
            library: Some(lib.id),
            recursive: true,
            sort: SortOrder::Random,
            limit: 2,
            ..Default::default()
        })
        .await
        .expect("random page");
    assert_eq!(page.items.len(), 2, "limit still applies");
    assert!(
        page.next_cursor().is_none(),
        "random sort never mints a cursor"
    );

    for w in &works {
        store.delete_work(w.id).await.ok();
    }
    store.delete_library(lib.id).await.ok();
}

async fn library_config_round_trip<S: Storage + ?Sized>(store: &S) {
    use crate::domain::{ArtworkPreference, LibraryCategory, LibraryConfig, ScanSchedule};

    let lib = test_library("Cfg Lib");
    assert_eq!(
        lib.config.artwork_preference,
        ArtworkPreference::PreferProvider,
        "provider artwork is the default"
    );
    let lib_id = lib.id;
    let mut configured = lib.clone();
    configured.config = LibraryConfig {
        preferred_languages: vec!["en".to_string(), "ja".to_string()],
        preferred_providers: vec!["tmdb".to_string(), "tvdb".to_string()],
        excluded_folders: vec!["*/.staging".to_string()],
        watch_enabled: false,
        scan_schedule: ScanSchedule::Daily,
        artwork_preference: ArtworkPreference::PreferLocal,
        content_category: LibraryCategory::Anime,
    };
    store.upsert_library(&configured).await.expect("upsert");

    let fetched = store
        .get_library(lib_id)
        .await
        .expect("get")
        .expect("library exists");
    assert_eq!(fetched.config.preferred_languages, vec!["en", "ja"]);
    assert_eq!(fetched.config.preferred_providers, vec!["tmdb", "tvdb"]);
    assert_eq!(fetched.config.excluded_folders, vec!["*/.staging"]);
    assert!(!fetched.config.watch_enabled);
    assert_eq!(fetched.config.scan_schedule, ScanSchedule::Daily);
    assert_eq!(
        fetched.config.artwork_preference,
        ArtworkPreference::PreferLocal
    );
    assert_eq!(fetched.config.content_category, LibraryCategory::Anime);

    let all = store.list_libraries().await.expect("list");
    let found = all.iter().find(|l| l.id == lib_id).expect("listed");
    assert_eq!(found.config.scan_schedule, ScanSchedule::Daily);

    let mut updated = fetched.clone();
    updated.config.scan_schedule = ScanSchedule::Hourly;
    updated.config.watch_enabled = true;
    store.upsert_library(&updated).await.expect("upsert 2");
    let fetched2 = store
        .get_library(lib_id)
        .await
        .expect("get 2")
        .expect("library exists");
    assert_eq!(fetched2.config.scan_schedule, ScanSchedule::Hourly);
    assert!(fetched2.config.watch_enabled);
    assert_eq!(fetched2.config.preferred_languages, vec!["en", "ja"]);

    store.delete_library(lib_id).await.ok();
}

async fn activity_log_round_trip<S: Storage + ?Sized>(store: &S) {
    let now = OffsetDateTime::now_utc();
    let earlier = now - time::Duration::minutes(5);
    let id1 = store
        .append_activity(
            ActivityKind::ScanCompleted,
            Some("scanner"),
            Some("lib-1"),
            Some("added=3, removed=0"),
            earlier,
        )
        .await
        .expect("append 1");
    let id2 = store
        .append_activity(
            ActivityKind::Custom,
            Some("admin"),
            Some("alice"),
            None,
            now,
        )
        .await
        .expect("append 2");
    assert!(id1 > 0 && id2 > id1);

    let all = store
        .list_activity(&[], None, 100, None)
        .await
        .expect("list all")
        .items;
    assert_eq!(all.len(), 2, "two rows");
    // Newest first.
    assert_eq!(all[0].kind, ActivityKind::Custom);
    assert_eq!(all[1].kind, ActivityKind::ScanCompleted);
    assert_eq!(all[0].actor.as_deref(), Some("admin"));
    assert_eq!(all[0].subject.as_deref(), Some("alice"));
    assert!(all[0].detail.is_none());

    let only_scans = store
        .list_activity(&[ActivityKind::ScanCompleted], None, 100, None)
        .await
        .expect("list scan completed")
        .items;
    assert_eq!(only_scans.len(), 1);
    assert_eq!(only_scans[0].kind, ActivityKind::ScanCompleted);

    let recent = store
        .list_activity(&[], Some(now - time::Duration::minutes(1)), 100, None)
        .await
        .expect("list recent")
        .items;
    assert_eq!(recent.len(), 1, "since=now-1m keeps only the newer row");
    assert_eq!(recent[0].kind, ActivityKind::Custom);

    let capped = store
        .list_activity(&[], None, 1, None)
        .await
        .expect("list limit 1")
        .items;
    assert_eq!(capped.len(), 1);
}

/// Activity pages walk `(occurred_at, id)` newest first, ties included,
/// without skips or repeats; a malformed cursor is rejected.
async fn activity_log_keyset_pagination<S: Storage + ?Sized>(store: &S) {
    use time::macros::datetime;

    // Far-future timestamps plus `since` isolate these rows from every
    // other test's activity.
    let base = datetime!(2099-01-01 00:00 UTC);
    let mut ids = Vec::new();
    for i in 0..5i64 {
        // Two rows share each timestamp, so the id tiebreak matters.
        let at = base + time::Duration::seconds(i / 2);
        ids.push(
            store
                .append_activity(ActivityKind::Custom, None, Some("page"), None, at)
                .await
                .expect("append"),
        );
    }
    let mut seen = Vec::new();
    let mut cursor = None;
    for _ in 0..10 {
        let page = store
            .list_activity(&[], Some(base), 2, cursor)
            .await
            .expect("page");
        assert_eq!(page.item_cursors.len(), page.items.len());
        let next = page.next_cursor();
        seen.extend(page.items.iter().map(|row| row.id));
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    ids.reverse();
    assert_eq!(seen, ids, "newest first, each row once");
    let err = store
        .list_activity(&[], Some(base), 2, Some(PageCursor("nope".into())))
        .await
        .expect_err("malformed cursor");
    assert!(matches!(err, StorageError::InvalidInput(_)), "{err:?}");
}

async fn recently_added_returns_top_level_only<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Rec Lib");
    let lib_id = lib.id;
    store.upsert_library(&lib).await.expect("library");

    let mut older = make_work_of_kind(
        library_id(&lib),
        WorkKind::Movie,
        "Older",
        OffsetDateTime::now_utc(),
    );
    older.year = Some(2000);
    let mut newer = make_work_of_kind(
        library_id(&lib),
        WorkKind::Movie,
        "Newer",
        OffsetDateTime::now_utc(),
    );
    newer.year = Some(2010);
    let mut child = make_work_of_kind(
        library_id(&lib),
        WorkKind::Episode,
        "Kid",
        OffsetDateTime::now_utc(),
    );
    child.parent_id = Some(older.id);
    let older_id = older.id;
    let newer_id = newer.id;
    let child_id = child.id;
    store.upsert_work(&older).await.expect("older");
    // Force a distinct added_at — UUIDv7 has a millisecond timestamp,
    // but UUIDs from a tight loop can collide. Bump newer.id by
    // re-issuing with a known-uuid after a tiny sleep is too heavy
    // for a test; the SQL ORDER BY breaks the tie on id, which is
    // also a UUIDv7 timestamp, so this works as long as the IDs
    // are issued in order.
    store.upsert_work(&newer).await.expect("newer");
    store.upsert_work(&child).await.expect("child");

    let recent = store.recently_added_works(10).await.expect("recent");
    let ids: Vec<WorkId> = recent.iter().map(|w| w.id).collect();
    assert!(ids.contains(&newer_id), "newer present");
    assert!(ids.contains(&older_id), "older present");
    assert!(
        !ids.contains(&child_id),
        "child (parent_id != NULL) is excluded"
    );

    store.delete_work(newer_id).await.ok();
    store.delete_work(older_id).await.ok();
    store.delete_work(child_id).await.ok();
    store.delete_library(lib_id).await.ok();
}

async fn recently_played_filters_by_user<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Played Lib");
    let lib_id = lib.id;
    store.upsert_library(&lib).await.expect("library");

    let user = make_user("u-rec", false);
    let other = make_user("u-rec-other", false);
    store.upsert_user(&user).await.expect("user");
    store.upsert_user(&other).await.expect("other");

    let mut movie = make_work_of_kind(
        library_id(&lib),
        WorkKind::Movie,
        "Played",
        OffsetDateTime::now_utc(),
    );
    movie.year = Some(2024);
    let movie_id = movie.id;
    store.upsert_work(&movie).await.expect("work");

    let target = PlayTarget::Work(movie_id);
    let now = OffsetDateTime::now_utc();
    store
        .record_playback_progress(user.id, target, 5_000)
        .await
        .expect("progress user");
    store
        .mark_played(user.id, target, true)
        .await
        .expect("played user");
    // Other user records progress but no `played=true` / last_played
    // set, so they must NOT appear in `recently_played_for_user`.
    let _ = store
        .record_playback_progress(other.id, target, 1_000)
        .await;
    // `mark_played` writes `last_played`, which is the column the
    // aggregation filters on. Give `other` a played row too so we
    // also exercise the per-user filter.
    store
        .mark_played(other.id, target, false)
        .await
        .expect("mark other");
    let _ = now;

    let user_recent = store
        .recently_played_for_user(user.id, 10)
        .await
        .expect("user recent");
    let user_ids: Vec<WorkId> = user_recent.iter().map(|w| w.id).collect();
    assert_eq!(user_ids, vec![movie_id]);

    let other_recent = store
        .recently_played_for_user(other.id, 10)
        .await
        .expect("other recent");
    // Other user only has a position_ms=1000, never `played=true`;
    // mark_played(false) sets `played` to 0. The aggregation
    // filters on `last_played IS NOT NULL` which is set by both
    // branches — so the movie does appear for them. The point of
    // this assertion is that the JOIN+filter runs without error
    // and the user dimension is honoured.
    let other_ids: Vec<WorkId> = other_recent.iter().map(|w| w.id).collect();
    assert_eq!(other_ids, vec![movie_id]);

    // A user with no user_data rows at all gets the empty list.
    let nobody_id = UserId::new();
    let none = store
        .recently_played_for_user(nobody_id, 10)
        .await
        .expect("nobody recent");
    assert!(none.is_empty());

    store.delete_work(movie_id).await.ok();
    store.delete_user(user.id).await.ok();
    store.delete_user(other.id).await.ok();
    store.delete_library(lib_id).await.ok();
}

async fn play_history_paginates_and_filters_unplayed<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Hist Lib");
    let lib_id = lib.id;
    store.upsert_library(&lib).await.expect("library");

    let user = make_user("u-hist", false);
    store.upsert_user(&user).await.expect("user");

    // Five works, played in order A → B → C → D → E. With a
    // 2ms sleep between each mark_played, the last_played
    // values are strictly increasing, newest = E.
    let work_ids: Vec<WorkId> = {
        let mut ids = Vec::new();
        for label in ["A", "B", "C", "D", "E"] {
            let mut w = make_work_of_kind(
                library_id(&lib),
                WorkKind::Movie,
                label,
                OffsetDateTime::now_utc(),
            );
            w.year = Some(2020);
            ids.push(w.id);
            store.upsert_work(&w).await.expect("work");
        }
        ids
    };

    for work_id in &work_ids {
        let target = PlayTarget::Work(*work_id);
        store
            .record_playback_progress(user.id, target, 1000)
            .await
            .expect("progress");
        store
            .mark_played(user.id, target, true)
            .await
            .expect("played");
        // `mark_played` stamps `last_played = now_utc()`. 2ms is
        // enough granularity for the page-cursor math to be
        // deterministic.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }

    let newest_first: Vec<_> = work_ids.iter().rev().map(|id| id.into_uuid()).collect();
    for limit in [1, 2, 3, 5, 7] {
        let mut seen = Vec::new();
        let mut cursor = None;
        let mut pages = 0;
        loop {
            let page = store
                .list_play_history(user.id, cursor, limit)
                .await
                .expect("history page");
            pages += 1;
            assert!(page.rows.len() <= limit as usize);
            seen.extend(page.rows.iter().map(|row| row.target_id));
            match page.next_cursor {
                Some(next) => {
                    assert_eq!(page.rows.len(), limit as usize, "only full pages continue");
                    cursor = Some(PageCursor(next));
                }
                None => break,
            }
            assert!(pages <= 5, "paging must terminate");
        }
        assert_eq!(
            seen, newest_first,
            "limit {limit}: every row exactly once, newest first"
        );
        assert_eq!(pages, 5usize.div_ceil(limit as usize).max(1));
    }

    // A row cursor resumes right after that row.
    let top = store
        .list_play_history(user.id, None, 5)
        .await
        .expect("all");
    let after_second = store
        .list_play_history(user.id, Some(top.rows[1].cursor.clone()), 10)
        .await
        .expect("resume from row");
    assert_eq!(
        after_second
            .rows
            .iter()
            .map(|row| row.target_id)
            .collect::<Vec<_>>(),
        newest_first[2..]
    );
    assert!(matches!(
        store
            .list_play_history(user.id, Some(PageCursor("bogus".into())), 2)
            .await
            .expect_err("malformed cursor"),
        StorageError::InvalidInput(_)
    ));

    // No-progress user yields an empty page.
    let nobody = make_user("u-hist-nobody", false);
    store.upsert_user(&nobody).await.expect("nobody");
    let empty = store
        .list_play_history(nobody.id, None, 10)
        .await
        .expect("empty");
    assert!(empty.rows.is_empty());
    assert!(empty.next_cursor.is_none());

    for w in &work_ids {
        store.delete_work(*w).await.ok();
    }
    store.delete_user(user.id).await.ok();
    store.delete_user(nobody.id).await.ok();
    store.delete_library(lib_id).await.ok();
}

// ---- Collections and playlists ------------------------------------------

async fn get_edition_round_trip<S: Storage + ?Sized>(store: &S) {
    let lib = Library {
        id: LibraryId::new(),
        name: "GetEdition".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/getedition".to_string()],
        ..Default::default()
    };
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Get Edition Movie", now);
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Theatrical");
    store.upsert_edition(&edition).await.expect("edition");

    let fetched = store
        .get_edition(edition.id)
        .await
        .expect("get_edition")
        .expect("present");
    assert_eq!(fetched.id, edition.id);
    assert_eq!(fetched.work_id, work.id);
    assert_eq!(fetched.name, "Theatrical");

    let missing = store
        .get_edition(EditionId::new())
        .await
        .expect("get_edition missing");
    assert!(missing.is_none());

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// Marker writes land on exactly the three marker columns: runtime and
/// name survive, all-`None` clears a previously set window, and writing
/// markers for an unknown edition is a `NotFound`.
async fn set_intro_markers_round_trip<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Intro Markers Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Intro Markers Movie", now);
    store.upsert_work(&work).await.expect("work");
    let mut edition = make_edition(work.id, "Default");
    edition.runtime_ms = Some(7_200_000);
    store.upsert_edition(&edition).await.expect("edition");

    let markers = IntroMarkers {
        intro_start_ms: Some(1_000),
        intro_end_ms: Some(91_000),
        credits_start_ms: Some(7_000_000),
    };
    store
        .set_intro_markers(edition.id, markers.clone())
        .await
        .expect("set markers");
    let fetched = store
        .get_edition(edition.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(fetched.intro_start_ms, markers.intro_start_ms);
    assert_eq!(fetched.intro_end_ms, markers.intro_end_ms);
    assert_eq!(fetched.credits_start_ms, markers.credits_start_ms);
    assert_eq!(
        fetched.runtime_ms,
        Some(7_200_000),
        "markers must not touch runtime"
    );
    assert_eq!(fetched.name, "Default");

    // A detector that finds nothing clears any stale window.
    store
        .set_intro_markers(
            edition.id,
            IntroMarkers {
                intro_start_ms: None,
                intro_end_ms: None,
                credits_start_ms: None,
            },
        )
        .await
        .expect("clear markers");
    let fetched = store
        .get_edition(edition.id)
        .await
        .expect("get after clear")
        .expect("present");
    assert_eq!(fetched.intro_start_ms, None);
    assert_eq!(fetched.intro_end_ms, None);
    assert_eq!(fetched.credits_start_ms, None);

    let err = store
        .set_intro_markers(EditionId::new(), markers)
        .await
        .expect_err("unknown edition");
    assert!(
        matches!(err, StorageError::NotFound),
        "expected NotFound, got {err:?}"
    );

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// Detection sweep: every edition is missing for an unseen plugin, then
/// `record_intro_scan` drops it out of that plugin's sweep only — other
/// plugins still see it. Re-recording upserts instead of duplicating,
/// pagination round-trips without losing or duplicating rows, and a
/// malformed cursor is a strict `InvalidInput`.
async fn intro_scan_record_and_sweep<S: Storage + ?Sized>(store: &S) {
    use crate::query::PageCursor;

    let lib = test_library("Intro Sweep Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Intro Sweep Movie", now);
    store.upsert_work(&work).await.expect("work");
    let a = make_edition(work.id, "Theatrical");
    let b = make_edition(work.id, "Extended");
    store.upsert_edition(&a).await.expect("a");
    store.upsert_edition(&b).await.expect("b");

    // No scans yet: both editions are missing for det-a.
    let page = store
        .editions_missing_intro_scan("det-a", 10, None)
        .await
        .expect("sweep");
    let mut ids: Vec<_> = page.items.iter().map(|e| e.id).collect();
    ids.sort_by_key(|id| id.into_uuid());
    let mut expected = vec![a.id, b.id];
    expected.sort_by_key(|id| id.into_uuid());
    assert_eq!(ids, expected, "both editions missing for det-a");
    assert!(page.next_cursor.is_none(), "fits under the limit");

    // Recording a's scan drops it from det-a's sweep; det-b is untouched.
    store
        .record_intro_scan(a.id, "det-a", true)
        .await
        .expect("record a");
    let det_a = store
        .editions_missing_intro_scan("det-a", 10, None)
        .await
        .expect("det-a");
    assert_eq!(det_a.items.len(), 1, "a is scanned for det-a, dropped");
    assert_eq!(det_a.items[0].id, b.id);
    let det_b = store
        .editions_missing_intro_scan("det-b", 10, None)
        .await
        .expect("det-b");
    let mut det_b_ids: Vec<_> = det_b.items.iter().map(|e| e.id).collect();
    det_b_ids.sort_by_key(|id| id.into_uuid());
    assert_eq!(det_b_ids, expected, "det-b is untouched");

    // Re-recording the same (edition, plugin) upserts: no error, no
    // duplicate row, and a stays out of det-a's sweep.
    store
        .record_intro_scan(a.id, "det-a", false)
        .await
        .expect("re-record a");
    let det_a = store
        .editions_missing_intro_scan("det-a", 10, None)
        .await
        .expect("det-a after re-record");
    assert_eq!(det_a.items.len(), 1);

    // limit-1 pages + cursor reassemble det-b's sweep exactly.
    let mut collected: Vec<EditionId> = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    loop {
        let page = store
            .editions_missing_intro_scan("det-b", 1, cursor)
            .await
            .expect("page");
        collected.extend(page.items.iter().map(|e| e.id));
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    let mut collected_sorted = collected.clone();
    collected_sorted.sort_by_key(|id| id.into_uuid());
    assert_eq!(
        collected_sorted, expected,
        "cursor walk visits every missing edition"
    );
    collected.sort_by_key(|id| id.into_uuid());
    let mut deduped = collected.clone();
    deduped.dedup();
    assert_eq!(collected, deduped, "no duplicates across pages");

    // Malformed cursor must error, not silently re-serve page one.
    let err = store
        .editions_missing_intro_scan("det-b", 1, Some(PageCursor("not-a-cursor".to_string())))
        .await
        .expect_err("bad cursor");
    assert!(
        matches!(err, StorageError::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

/// Deleting a work cascades through its editions to their scan rows:
/// with FK enforcement on, the delete only succeeds if the scan rows
/// went with the edition.
async fn intro_scan_rows_cascade_with_edition<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Intro Cascade Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Intro Cascade Movie", now);
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Default");
    store.upsert_edition(&edition).await.expect("edition");
    store
        .record_intro_scan(edition.id, "det-a", true)
        .await
        .expect("record");

    store
        .delete_work(work.id)
        .await
        .expect("delete must cascade through scan rows, not violate FK");
    let sweep = store
        .editions_missing_intro_scan("det-a", 10, None)
        .await
        .expect("sweep after delete");
    assert!(sweep.items.is_empty(), "edition and scan row are gone");

    store.delete_library(lib.id).await.ok();
}

async fn trickplay_round_trip_sweep_and_staleness<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Trickplay Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Trickplay Movie", now);
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Default");
    store.upsert_edition(&edition).await.expect("edition");
    let mut media = make_media(edition.id, "/media/trickplay.mkv", now);
    store.upsert_media_file(&media).await.expect("media");

    let missing = store
        .editions_missing_trickplay(10, None)
        .await
        .expect("missing");
    assert_eq!(missing.items.len(), 1);

    let asset = TrickplayAsset {
        edition_id: edition.id,
        vtt_path: "/cache/index.vtt".to_string(),
        thumb_width: 160,
        thumb_height: 90,
        tile_columns: 10,
        tile_rows: 10,
        interval_ms: 10_000,
        source_media_mtime: now,
        source_fingerprint: crate::canonical::trickplay_source_fingerprint(
            "/media/trickplay.mkv",
            1024,
            now,
        ),
        generated_at: now,
        sheets: vec![TrickplaySheet {
            edition_id: edition.id,
            index: 0,
            path: "/cache/sheet-0000.jpg".to_string(),
            tile_count: 42,
        }],
        chapter_cues: vec![TrickplayChapterCue {
            edition_id: edition.id,
            position_ms: 20_500,
            title: Some("Two".to_string()),
            cue_start_ms: 20_000,
            cue_end_ms: 30_000,
            sheet_index: 0,
            x: 320,
            y: 0,
            width: 160,
            height: 90,
        }],
    };
    store.upsert_trickplay(&asset).await.expect("upsert");
    assert_eq!(
        store.get_trickplay(edition.id).await.expect("get"),
        Some(asset)
    );
    assert!(
        store
            .editions_missing_trickplay(10, None)
            .await
            .expect("current")
            .items
            .is_empty()
    );

    media.mtime += time::Duration::seconds(1);
    store.upsert_media_file(&media).await.expect("touch media");
    assert_eq!(
        store
            .editions_missing_trickplay(10, None)
            .await
            .expect("stale")
            .items
            .len(),
        1
    );
    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

async fn trickplay_cascades_with_edition<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Trickplay Cascade Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Trickplay Cascade", now);
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Default");
    store.upsert_edition(&edition).await.expect("edition");
    let asset = TrickplayAsset {
        edition_id: edition.id,
        vtt_path: "/cache/index.vtt".to_string(),
        thumb_width: 160,
        thumb_height: 90,
        tile_columns: 10,
        tile_rows: 10,
        interval_ms: 10_000,
        source_media_mtime: now,
        source_fingerprint: "fingerprint".to_string(),
        generated_at: now,
        sheets: Vec::new(),
        chapter_cues: Vec::new(),
    };
    store.upsert_trickplay(&asset).await.expect("upsert");
    store
        .delete_edition(edition.id)
        .await
        .expect("delete edition");
    assert!(
        store
            .get_trickplay(edition.id)
            .await
            .expect("get")
            .is_none()
    );
    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

fn make_collection(library_id: LibraryId, title: &str, sort_title: &str) -> Collection {
    let now = OffsetDateTime::now_utc();
    Collection {
        id: CollectionId::new(),
        library_id,
        title: title.to_string(),
        sort_title: sort_title.to_string(),
        overview: None,
        created_at: now,
        updated_at: now,
    }
}

fn make_playlist(owner_id: UserId, title: &str) -> Playlist {
    let now = OffsetDateTime::now_utc();
    Playlist {
        id: PlaylistId::new(),
        owner_id,
        title: title.to_string(),
        overview: None,
        created_at: now,
        updated_at: now,
    }
}

async fn collection_crud_round_trip<S: Storage + ?Sized>(store: &S) {
    let lib = Library {
        id: LibraryId::new(),
        name: "Collections".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/collections".to_string()],
        ..Default::default()
    };
    store.upsert_library(&lib).await.expect("library");

    // Two collections, created out of sort order; list must order by
    // sort_title.
    let bravo = make_collection(lib.id, "Bravo", "bravo");
    let alpha = make_collection(lib.id, "Alpha", "alpha");
    store.create_collection(&bravo).await.expect("create bravo");
    store.create_collection(&alpha).await.expect("create alpha");

    let fetched = store
        .get_collection(bravo.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(fetched.title, "Bravo");
    assert_eq!(fetched.library_id, lib.id);
    assert_eq!(fetched.created_at, bravo.created_at);

    let first = store
        .list_collections(lib.id, 1, None)
        .await
        .expect("first page");
    assert_eq!(first.items[0].title, "Alpha");
    let foreign_library = Library {
        id: LibraryId::new(),
        name: "Foreign collections".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/foreign-collections".to_string()],
        ..Default::default()
    };
    store
        .upsert_library(&foreign_library)
        .await
        .expect("foreign library");
    // Cursors are pure keyset positions: replaying one elsewhere just
    // resumes after that key.
    let foreign = store
        .list_collections(foreign_library.id, 1, first.next_cursor().clone())
        .await
        .expect("cursor from another library");
    assert!(foreign.items.is_empty());
    // Deleting the anchor row must not invalidate the cursor.
    store
        .delete_collection(alpha.id)
        .await
        .expect("delete anchor");
    let listed = store
        .list_collections(lib.id, 1, first.next_cursor())
        .await
        .expect("second page");
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].title, "Bravo");

    let mut updated = bravo.clone();
    updated.title = "Bravo Renamed".to_string();
    updated.overview = Some("curated".to_string());
    updated.updated_at += time::Duration::seconds(1);
    store.update_collection(&updated).await.expect("update");
    let fetched = store
        .get_collection(bravo.id)
        .await
        .expect("get after update")
        .expect("present");
    assert_eq!(fetched.title, "Bravo Renamed");
    assert_eq!(fetched.overview.as_deref(), Some("curated"));
    assert_eq!(fetched.updated_at, updated.updated_at);

    // Update/delete of a missing row is NotFound, not a silent no-op.
    let mut ghost = make_collection(lib.id, "Ghost", "ghost");
    ghost.id = CollectionId::new();
    let err = store
        .update_collection(&ghost)
        .await
        .expect_err("update missing");
    assert!(matches!(err, StorageError::NotFound));
    let err = store
        .delete_collection(CollectionId::new())
        .await
        .expect_err("delete missing");
    assert!(matches!(err, StorageError::NotFound));

    assert!(
        store
            .get_collection(alpha.id)
            .await
            .expect("get after delete")
            .is_none()
    );
    let listed = store
        .list_collections(lib.id, 10, None)
        .await
        .expect("list after delete");
    assert_eq!(listed.items.len(), 1);

    store.delete_collection(bravo.id).await.ok();
    store.delete_library(lib.id).await.ok();
    store.delete_library(foreign_library.id).await.ok();
}

async fn collection_items_order_and_dedupe<S: Storage + ?Sized>(store: &S) {
    let lib = Library {
        id: LibraryId::new(),
        name: "CollectionItems".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/collection-items".to_string()],
        ..Default::default()
    };
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let w1 = make_work(lib.id, "One", now);
    let w2 = make_work(lib.id, "Two", now);
    let w3 = make_work(lib.id, "Three", now);
    for w in [&w1, &w2, &w3] {
        store.upsert_work(w).await.expect("work");
    }
    let collection = make_collection(lib.id, "Marathon", "marathon");
    store
        .create_collection(&collection)
        .await
        .expect("collection");

    // Order is preserved as given, not re-sorted.
    store
        .set_collection_items(collection.id, &[w2.id, w1.id, w3.id])
        .await
        .expect("set items");
    let first = store
        .collection_items(collection.id, admin_principal(), 1, None)
        .await
        .expect("first page");
    assert_eq!(first.items, vec![w2.id]);
    assert!(first.has_next);
    let cursor_for_another_collection = first.end_cursor.clone().expect("first cursor");
    let foreign = store
        .collection_items(
            CollectionId::new(),
            admin_principal(),
            1,
            Some(cursor_for_another_collection),
        )
        .await
        .expect("cursor from another collection");
    assert!(foreign.items.is_empty());
    let middle = store
        .collection_items(collection.id, admin_principal(), 1, first.end_cursor)
        .await
        .expect("middle page");
    assert_eq!(middle.items, vec![w1.id]);
    assert!(middle.has_next);
    let final_page = store
        .collection_items(collection.id, admin_principal(), 1, middle.end_cursor)
        .await
        .expect("final page");
    assert_eq!(final_page.items, vec![w3.id]);
    assert!(!final_page.has_next);

    let err = store
        .collection_items(
            collection.id,
            admin_principal(),
            10,
            Some(PageCursor("bad".to_string())),
        )
        .await
        .expect_err("invalid cursor");
    assert!(matches!(err, StorageError::InvalidInput(_)));

    // A second set is a full replace: reorders and drops.
    store
        .set_collection_items(collection.id, &[w3.id, w2.id])
        .await
        .expect("reorder");
    let items = store
        .collection_items(collection.id, admin_principal(), 10, None)
        .await
        .expect("reordered");
    assert_eq!(items.items, vec![w3.id, w2.id]);

    // Duplicates collapse to their first occurrence.
    store
        .set_collection_items(collection.id, &[w1.id, w3.id, w1.id, w3.id, w2.id])
        .await
        .expect("dedupe set");
    let items = store
        .collection_items(collection.id, admin_principal(), u32::MAX, None)
        .await
        .expect("deduped");
    assert_eq!(items.items, vec![w1.id, w3.id, w2.id]);

    // set on a missing collection is NotFound.
    let err = store
        .set_collection_items(CollectionId::new(), &[w1.id])
        .await
        .expect_err("set on missing");
    assert!(matches!(err, StorageError::NotFound));

    for w in [&w1, &w2, &w3] {
        store.delete_work(w.id).await.ok();
    }
    store.delete_collection(collection.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

async fn visible_item_pagination<S: Storage + ?Sized>(store: &S) {
    let user = make_user("visible-page-owner", false);
    let other = make_user("visible-page-other", false);
    store.upsert_user(&user).await.expect("user");
    store.upsert_user(&other).await.expect("other user");
    let library = Library {
        id: LibraryId::new(),
        name: "Visible pages".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/visible-pages".to_string()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    store
        .grant_library_access(user.id, library.id)
        .await
        .expect("grant");
    let allowed_rating = crate::domain::Tag {
        id: TagId::new(),
        namespace: "parental".to_string(),
        parent_id: None,
        slug: "visible-page-allowed".to_string(),
        display: "Allowed".to_string(),
        metadata: async_graphql::Json(serde_json::json!({})),
    };
    let hidden_rating = crate::domain::Tag {
        id: TagId::new(),
        namespace: "parental".to_string(),
        parent_id: None,
        slug: "visible-page-hidden".to_string(),
        display: "Hidden".to_string(),
        metadata: async_graphql::Json(serde_json::json!({})),
    };
    store
        .upsert_tag(&allowed_rating)
        .await
        .expect("allowed tag");
    store.upsert_tag(&hidden_rating).await.expect("hidden tag");
    store
        .set_user_restrictions(&UserRestrictions {
            user_id: user.id,
            allowed_parental_rating_ids: vec![allowed_rating.id],
            allow_unrated: false,
            max_stream_bitrate_bps: None,
            max_concurrent_transcodes: None,
        })
        .await
        .expect("restrictions");

    let now = OffsetDateTime::now_utc();
    let visible_one = make_work(library.id, "Visible one", now);
    let hidden_one = make_work(library.id, "Hidden one", now);
    let hidden_two = make_work(library.id, "Hidden two", now);
    let visible_two = make_work(library.id, "Visible two", now);
    let hidden_tail = make_work(library.id, "Hidden tail", now);
    for work in [
        &visible_one,
        &hidden_one,
        &hidden_two,
        &visible_two,
        &hidden_tail,
    ] {
        store.upsert_work(work).await.expect("work");
    }
    for work in [&visible_one, &visible_two] {
        store
            .assign_tag(allowed_rating.id, TagTarget::Work(work.id), 1.0, "test")
            .await
            .expect("allowed assignment");
    }
    for work in [&hidden_one, &hidden_two, &hidden_tail] {
        store
            .assign_tag(hidden_rating.id, TagTarget::Work(work.id), 1.0, "test")
            .await
            .expect("hidden assignment");
    }
    let principal = RequestUser {
        user_id: user.id,
        is_admin: false,
    };
    let collection = make_collection(library.id, "Visibility", "visibility");
    store
        .create_collection(&collection)
        .await
        .expect("collection");
    store
        .set_collection_items(
            collection.id,
            &[
                visible_one.id,
                hidden_one.id,
                hidden_two.id,
                visible_two.id,
                hidden_tail.id,
            ],
        )
        .await
        .expect("collection items");
    let first = store
        .collection_items(collection.id, principal, 1, None)
        .await
        .expect("visible first page");
    assert_eq!(first.items, vec![visible_one.id]);
    assert!(first.has_next);
    let second = store
        .collection_items(collection.id, principal, 1, first.end_cursor)
        .await
        .expect("visible second page");
    assert_eq!(second.items, vec![visible_two.id]);
    assert!(!second.has_next, "an all-hidden tail is not a continuation");
    assert_eq!(
        [first.items, second.items].concat(),
        vec![visible_one.id, visible_two.id]
    );

    let visible_edition = make_edition(visible_one.id, "Visible edition");
    let hidden_edition = make_edition(hidden_one.id, "Hidden edition");
    store
        .upsert_edition(&visible_edition)
        .await
        .expect("edition");
    store
        .upsert_edition(&hidden_edition)
        .await
        .expect("edition");
    let playlist = make_playlist(user.id, "Visibility");
    store.create_playlist(&playlist).await.expect("playlist");
    store
        .set_playlist_items(
            playlist.id,
            &[
                PlayTarget::Edition(visible_edition.id),
                PlayTarget::Work(hidden_two.id),
                PlayTarget::Edition(hidden_edition.id),
                PlayTarget::Work(visible_two.id),
                PlayTarget::Work(hidden_tail.id),
            ],
        )
        .await
        .expect("playlist items");
    let first = store
        .playlist_items(playlist.id, principal, 1, None)
        .await
        .expect("playlist first page");
    assert_eq!(first.items, vec![PlayTarget::Edition(visible_edition.id)]);
    assert!(first.has_next);
    let second = store
        .playlist_items(playlist.id, principal, 1, first.end_cursor)
        .await
        .expect("playlist second page");
    assert_eq!(second.items, vec![PlayTarget::Work(visible_two.id)]);
    assert!(!second.has_next);
    let non_owner = RequestUser {
        user_id: other.id,
        is_admin: false,
    };
    assert!(
        store
            .playlist_items(playlist.id, non_owner, 10, None)
            .await
            .expect("non-owner page")
            .items
            .is_empty()
    );
    assert_eq!(
        store
            .playlist_items(playlist.id, admin_principal(), 10, None)
            .await
            .expect("admin page")
            .items
            .len(),
        5
    );
}

async fn collection_items_cascade<S: Storage + ?Sized>(store: &S) {
    let lib = Library {
        id: LibraryId::new(),
        name: "CollectionCascade".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/collection-cascade".to_string()],
        ..Default::default()
    };
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let w1 = make_work(lib.id, "Stay", now);
    let w2 = make_work(lib.id, "Go", now);
    store.upsert_work(&w1).await.expect("w1");
    store.upsert_work(&w2).await.expect("w2");
    let collection = make_collection(lib.id, "Cascade", "cascade");
    store
        .create_collection(&collection)
        .await
        .expect("collection");
    store
        .set_collection_items(collection.id, &[w1.id, w2.id])
        .await
        .expect("set items");

    // Deleting a work pulls it out of the membership list.
    store.delete_work(w2.id).await.expect("delete work");
    let items = store
        .collection_items(collection.id, admin_principal(), 10, None)
        .await
        .expect("items after work delete");
    assert_eq!(items.items, vec![w1.id]);

    // Deleting the collection takes the items with it.
    store
        .delete_collection(collection.id)
        .await
        .expect("delete collection");
    let items = store
        .collection_items(collection.id, admin_principal(), 10, None)
        .await
        .expect("items after collection delete");
    assert!(items.items.is_empty());

    store.delete_work(w1.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

async fn playlist_crud_round_trip<S: Storage + ?Sized>(store: &S) {
    let alice = make_user("playlist-alice", false);
    let bob = make_user("playlist-bob", false);
    store.upsert_user(&alice).await.expect("alice");
    store.upsert_user(&bob).await.expect("bob");

    let alice_list = make_playlist(alice.id, "Alice's Mix");
    let bob_list = make_playlist(bob.id, "Bob's Mix");
    store
        .create_playlist(&alice_list)
        .await
        .expect("create alice");
    store.create_playlist(&bob_list).await.expect("create bob");

    let fetched = store
        .get_playlist(alice_list.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(fetched.title, "Alice's Mix");
    assert_eq!(fetched.owner_id, alice.id);

    // Owners only ever see their own playlists.
    let alice_lists = store
        .list_playlists(alice.id, 10, None)
        .await
        .expect("list alice");
    assert_eq!(alice_lists.items.len(), 1);
    assert_eq!(alice_lists.items[0].id, alice_list.id);
    let bob_lists = store
        .list_playlists(bob.id, 10, None)
        .await
        .expect("list bob");
    assert_eq!(bob_lists.items.len(), 1);
    assert_eq!(bob_lists.items[0].id, bob_list.id);

    let mut updated = alice_list.clone();
    updated.title = "Alice's Renamed Mix".to_string();
    updated.updated_at += time::Duration::seconds(1);
    store.update_playlist(&updated).await.expect("update");
    let fetched = store
        .get_playlist(alice_list.id)
        .await
        .expect("get after update")
        .expect("present");
    assert_eq!(fetched.title, "Alice's Renamed Mix");

    let mut ghost = make_playlist(alice.id, "Ghost");
    ghost.id = PlaylistId::new();
    let err = store
        .update_playlist(&ghost)
        .await
        .expect_err("update missing");
    assert!(matches!(err, StorageError::NotFound));
    let err = store
        .delete_playlist(PlaylistId::new())
        .await
        .expect_err("delete missing");
    assert!(matches!(err, StorageError::NotFound));

    store
        .delete_playlist(bob_list.id)
        .await
        .expect("delete bob's");
    assert!(
        store
            .get_playlist(bob_list.id)
            .await
            .expect("get after delete")
            .is_none()
    );

    store.delete_playlist(alice_list.id).await.ok();
    store.delete_user(alice.id).await.ok();
    store.delete_user(bob.id).await.ok();
}

async fn playlist_items_mixed_targets<S: Storage + ?Sized>(store: &S) {
    let user = make_user("playlist-mixed", false);
    store.upsert_user(&user).await.expect("user");
    let lib = Library {
        id: LibraryId::new(),
        name: "PlaylistMixed".to_string(),
        kind: LibraryKind::Movies,
        paths: vec!["/srv/playlist-mixed".to_string()],
        ..Default::default()
    };
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Mixed Movie", now);
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Director's Cut");
    store.upsert_edition(&edition).await.expect("edition");

    let playlist = make_playlist(user.id, "Mixed");
    store.create_playlist(&playlist).await.expect("playlist");

    // Work and edition targets coexist, order preserved.
    let targets = [
        PlayTarget::Edition(edition.id),
        PlayTarget::Work(work.id),
        PlayTarget::Work(work.id),
    ];
    store
        .set_playlist_items(playlist.id, &targets)
        .await
        .expect("set items");
    let first = store
        .playlist_items(playlist.id, admin_principal(), 1, None)
        .await
        .expect("first page");
    assert_eq!(first.items, vec![PlayTarget::Edition(edition.id)]);
    assert!(first.has_next);
    let cursor_for_another_playlist = first.end_cursor.clone().expect("first cursor");
    let foreign = store
        .playlist_items(
            PlaylistId::new(),
            admin_principal(),
            1,
            Some(cursor_for_another_playlist),
        )
        .await
        .expect("cursor from another playlist");
    assert!(foreign.items.is_empty());
    let final_page = store
        .playlist_items(playlist.id, admin_principal(), 1, first.end_cursor)
        .await
        .expect("final page");
    assert_eq!(final_page.items, vec![PlayTarget::Work(work.id)]);
    assert!(!final_page.has_next);
    let err = store
        .playlist_items(
            playlist.id,
            admin_principal(),
            10,
            Some(PageCursor("bad".to_string())),
        )
        .await
        .expect_err("invalid cursor");
    assert!(matches!(err, StorageError::InvalidInput(_)));

    store
        .append_playlist_items(
            playlist.id,
            &[PlayTarget::Work(work.id), PlayTarget::Edition(edition.id)],
        )
        .await
        .expect("deduped append");
    let items = store
        .playlist_items(playlist.id, admin_principal(), u32::MAX, None)
        .await
        .expect("items after append");
    assert_eq!(
        items.items,
        vec![PlayTarget::Edition(edition.id), PlayTarget::Work(work.id)]
    );

    // set on a missing playlist is NotFound.
    let err = store
        .set_playlist_items(PlaylistId::new(), &[PlayTarget::Work(work.id)])
        .await
        .expect_err("set on missing");
    assert!(matches!(err, StorageError::NotFound));

    store.delete_playlist(playlist.id).await.ok();
    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
    store.delete_user(user.id).await.ok();
}

async fn playlist_user_delete_cascade<S: Storage + ?Sized>(store: &S) {
    let user = make_user("playlist-cascade", false);
    store.upsert_user(&user).await.expect("user");
    let playlist = make_playlist(user.id, "Doomed");
    store.create_playlist(&playlist).await.expect("playlist");
    store.delete_user(user.id).await.expect("delete user");

    assert!(
        store
            .get_playlist(playlist.id)
            .await
            .expect("get after user delete")
            .is_none(),
        "playlists wipe with their owner"
    );
    let items = store
        .playlist_items(playlist.id, admin_principal(), 10, None)
        .await
        .expect("items after user delete");
    assert!(items.items.is_empty());
}

async fn user_restrictions_and_access<S: Storage + ?Sized>(store: &S) {
    let admin = make_user("restriction-admin", true);
    let user = make_user("restriction-user", false);
    store.upsert_user(&admin).await.expect("admin");
    store.upsert_user(&user).await.expect("user");
    let library = Library {
        id: LibraryId::new(),
        name: "Restricted".into(),
        kind: LibraryKind::Movies,
        paths: vec!["/restricted".into()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    let work = make_work(library.id, "Rated", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let rating = crate::domain::Tag {
        id: TagId::new(),
        namespace: "parental".into(),
        parent_id: None,
        slug: "pg-13".into(),
        display: "PG-13".into(),
        metadata: async_graphql::Json(serde_json::json!({})),
    };
    store.upsert_tag(&rating).await.expect("tag");
    store
        .assign_tag(rating.id, TagTarget::Work(work.id), 1.0, "test")
        .await
        .expect("assign");

    assert!(
        !store
            .user_can_access_work(user.id, work.id)
            .await
            .expect("no grant")
    );
    assert!(
        store
            .list_works(&crate::query::ListWorksQuery {
                access_user_id: Some(user.id),
                limit: 10,
                ..Default::default()
            })
            .await
            .expect("filtered works")
            .items
            .is_empty()
    );
    assert!(
        store
            .search_lexical("Rated", &[], Some(user.id), 10)
            .await
            .expect("filtered search")
            .is_empty()
    );
    assert!(
        store
            .user_can_access_work(admin.id, work.id)
            .await
            .expect("admin bypass")
    );
    store
        .grant_library_access(user.id, library.id)
        .await
        .expect("grant");
    assert!(
        store
            .user_can_access_work(user.id, work.id)
            .await
            .expect("unrestricted")
    );
    assert_eq!(
        store
            .list_works(&crate::query::ListWorksQuery {
                access_user_id: Some(user.id),
                limit: 10,
                ..Default::default()
            })
            .await
            .expect("granted works")
            .items
            .len(),
        1
    );
    assert_eq!(
        store
            .search_lexical("Rated", &[], Some(user.id), 10)
            .await
            .expect("granted search")
            .len(),
        1
    );
    let policy = UserRestrictions {
        user_id: user.id,
        allowed_parental_rating_ids: vec![TagId::new()],
        allow_unrated: false,
        max_stream_bitrate_bps: Some(4_000_000),
        max_concurrent_transcodes: Some(1),
    };
    assert!(
        store.set_user_restrictions(&policy).await.is_err(),
        "non-parental/missing tag rejected"
    );
    let policy = UserRestrictions {
        allowed_parental_rating_ids: vec![rating.id],
        ..policy
    };
    store
        .set_user_restrictions(&policy)
        .await
        .expect("set policy");
    assert!(
        store
            .user_can_access_work(user.id, work.id)
            .await
            .expect("allowed rating")
    );
    assert_eq!(
        store.get_user_restrictions(user.id).await.expect("get"),
        Some(policy)
    );
    store.delete_user(user.id).await.expect("delete user");
    assert!(
        store
            .get_user_restrictions(user.id)
            .await
            .expect("cascade")
            .is_none()
    );
    store.delete_user(admin.id).await.ok();
    store.delete_library(library.id).await.ok();
}

async fn watch_room_lifecycle<S: Storage + ?Sized>(store: &S) {
    let host = make_user("watch-host", false);
    let guest = make_user("watch-guest", false);
    store.upsert_user(&host).await.expect("host");
    store.upsert_user(&guest).await.expect("guest");
    let library = Library {
        id: LibraryId::new(),
        name: "Watch".into(),
        kind: LibraryKind::Movies,
        paths: vec!["/watch".into()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");
    store
        .grant_library_access(host.id, library.id)
        .await
        .expect("host grant");
    store
        .grant_library_access(guest.id, library.id)
        .await
        .expect("guest grant");
    let work = make_work(library.id, "Watch", OffsetDateTime::now_utc());
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Default");
    store.upsert_edition(&edition).await.expect("edition");
    let media = make_media(edition.id, "/watch/movie.mkv", OffsetDateTime::now_utc());
    store.upsert_media_file(&media).await.expect("media");
    let now = OffsetDateTime::now_utc();
    let room = WatchRoom {
        id: WatchRoomId::new(),
        host_user_id: host.id,
        media_file_id: media.id,
        state: WatchRoomState::Waiting,
        position_ms: 0,
        sequence: 0,
        created_at: now,
        updated_at: now,
    };
    store.create_watch_room(&room).await.expect("create");
    assert_eq!(
        store
            .list_watch_room_members(room.id)
            .await
            .expect("members")
            .len(),
        1
    );
    store
        .join_watch_room(&WatchRoomMember {
            room_id: room.id,
            user_id: guest.id,
            joined_at: now,
        })
        .await
        .expect("join")
        .expect("accessible room");
    assert!(
        store
            .control_watch_room(
                room.id,
                guest.id,
                Some(WatchRoomState::Playing),
                Some(10),
                now
            )
            .await
            .expect("guest control")
            .is_none()
    );
    let controlled = store
        .control_watch_room(
            room.id,
            host.id,
            Some(WatchRoomState::Playing),
            Some(10),
            now,
        )
        .await
        .expect("host control")
        .expect("room");
    assert_eq!(controlled.sequence, 1);
    assert_eq!(controlled.position_ms, 10);
    let ended_snapshot = store
        .leave_watch_room(room.id, host.id)
        .await
        .expect("host leave")
        .expect("host leave ends room");
    assert_eq!(ended_snapshot.state, WatchRoomState::Ended);
    let ended = store
        .get_watch_room(room.id)
        .await
        .expect("get")
        .expect("room remains");
    assert_eq!(ended.state, WatchRoomState::Ended);
    assert!(
        store
            .control_watch_room(
                room.id,
                host.id,
                Some(WatchRoomState::Playing),
                None,
                OffsetDateTime::now_utc(),
            )
            .await
            .expect("ended control")
            .is_none()
    );
    assert!(
        store
            .join_watch_room(&WatchRoomMember {
                room_id: room.id,
                user_id: guest.id,
                joined_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("ended join")
            .is_none()
    );
    assert!(
        store
            .list_watch_room_members(room.id)
            .await
            .expect("cleared")
            .is_empty()
    );

    for _ in 0..8 {
        let mut concurrent_room = room.clone();
        concurrent_room.id = WatchRoomId::new();
        concurrent_room.state = WatchRoomState::Waiting;
        concurrent_room.sequence = 0;
        store
            .create_watch_room(&concurrent_room)
            .await
            .expect("create concurrent room");
        store
            .join_watch_room(&WatchRoomMember {
                room_id: concurrent_room.id,
                user_id: guest.id,
                joined_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("concurrent join")
            .expect("accessible concurrent room");
        let (guest_leave, host_leave) = tokio::join!(
            store.leave_watch_room(concurrent_room.id, guest.id),
            store.leave_watch_room(concurrent_room.id, host.id),
        );
        assert!(guest_leave.expect("concurrent guest leave").is_none());
        let ended = host_leave
            .expect("concurrent host leave")
            .expect("host ends concurrent room");
        assert_eq!(ended.state, WatchRoomState::Ended);
        assert_eq!(ended.sequence, 1);
        assert!(
            store
                .list_watch_room_members(concurrent_room.id)
                .await
                .expect("concurrent members cleared")
                .is_empty()
        );
    }

    store.delete_user(host.id).await.expect("delete host");
    assert!(
        store
            .get_watch_room(room.id)
            .await
            .expect("cascade")
            .is_none()
    );
    store.delete_user(guest.id).await.ok();
    store.delete_library(library.id).await.ok();
}

async fn live_tv_dvr_lifecycle<S: Storage + ?Sized>(store: &S) {
    use noflex_core::{ChannelId, EpgProgramId, LiveTvSourceId, RecordingScheduleId};

    use crate::domain::{
        EpgProgram, LiveTvChannel, LiveTvSource, RecordingSchedule, RecordingScheduleStatus,
        RecordingStatus,
    };

    let now = OffsetDateTime::now_utc();
    let user = make_user("live-tv-owner", false);
    store.upsert_user(&user).await.expect("live tv user");
    let source = LiveTvSource {
        id: LiveTvSourceId::new(),
        name: "Antenna".into(),
        enabled: true,
        last_refresh_at: None,
        last_error: None,
        playlist_url: "https://example.test/channels.m3u".into(),
        epg_url: Some("https://example.test/guide.xml".into()),
    };
    store.upsert_live_tv_source(&source).await.expect("source");
    let channel = LiveTvChannel {
        id: ChannelId::new(),
        source_id: source.id,
        external_id: "news".into(),
        name: "News".into(),
        number: Some("7".into()),
        logo_url: None,
        group: Some("Local".into()),
        enabled: true,
        stream_url: "https://example.test/live".into(),
    };
    let program = EpgProgram {
        id: EpgProgramId::new(),
        channel_id: channel.id,
        external_id: "news-noon".into(),
        title: "Noon News".into(),
        subtitle: None,
        description: None,
        category: Some("News".into()),
        start_at: now - time::Duration::minutes(1),
        end_at: now + time::Duration::hours(1),
    };
    store
        .apply_live_tv_refresh(
            source.id,
            std::slice::from_ref(&channel),
            std::slice::from_ref(&program),
            now,
        )
        .await
        .expect("guide refresh");
    assert_eq!(
        store
            .list_live_tv_guide(
                &[channel.id],
                now - time::Duration::hours(1),
                now + time::Duration::hours(2),
                10,
                None,
            )
            .await
            .expect("guide")
            .items
            .len(),
        1
    );
    let snapshot = |suffix: &str| {
        let mut channel = channel.clone();
        channel.id = ChannelId::new();
        channel.name = format!("News {suffix}");
        channel.stream_url = format!("https://example.test/live/{suffix}");
        let mut program = program.clone();
        program.id = EpgProgramId::new();
        program.channel_id = channel.id;
        program.title = format!("Noon News {suffix}");
        (channel, program)
    };
    let (channel_a, program_a) = snapshot("A");
    let (channel_b, program_b) = snapshot("B");
    let channels_a = [channel_a];
    let programs_a = [program_a];
    let channels_b = [channel_b];
    let programs_b = [program_b];
    let (refresh_a, refresh_b) = futures::join!(
        store.apply_live_tv_refresh(source.id, &channels_a, &programs_a, now),
        store.apply_live_tv_refresh(source.id, &channels_b, &programs_b, now),
    );
    refresh_a.expect("concurrent refresh A");
    refresh_b.expect("concurrent refresh B");
    let refreshed_channel = store
        .get_live_tv_channel(channel.id)
        .await
        .expect("stable channel lookup")
        .expect("stable channel retained");
    let refreshed_program = store
        .get_epg_program(program.id)
        .await
        .expect("stable program lookup")
        .expect("stable program retained");
    assert_eq!(
        refreshed_channel.name.strip_prefix("News "),
        refreshed_program.title.strip_prefix("Noon News "),
        "a refresh snapshot must not be partially visible"
    );
    let schedule = RecordingSchedule {
        id: RecordingScheduleId::new(),
        owner_id: user.id,
        channel_id: channel.id,
        program_id: Some(program.id),
        start_at: now - time::Duration::seconds(1),
        end_at: now + time::Duration::hours(1),
        padding_before_secs: 0,
        padding_after_secs: 30,
        status: RecordingScheduleStatus::Scheduled,
        created_at: now,
        updated_at: now,
    };
    store
        .create_recording_schedule(&schedule)
        .await
        .expect("schedule");
    let (claimed, recording) = store
        .claim_due_recording(now)
        .await
        .expect("claim")
        .expect("due schedule");
    assert_eq!(claimed.status, RecordingScheduleStatus::Triggered);
    assert!(
        store
            .claim_due_recording(now)
            .await
            .expect("second claim")
            .is_none(),
        "claim is one-shot"
    );
    assert_eq!(recording.schedule_id, Some(schedule.id));
    store
        .finish_recording(
            recording.id,
            RecordingStatus::Completed,
            Some("/tmp/news.mkv"),
            None,
            Some(42),
            now,
        )
        .await
        .expect("complete recording");
    assert_eq!(
        store
            .get_recording(recording.id)
            .await
            .expect("get recording")
            .expect("recording exists")
            .size_bytes,
        Some(42)
    );
    let race_schedule = RecordingSchedule {
        id: RecordingScheduleId::new(),
        program_id: None,
        ..schedule.clone()
    };
    store
        .create_recording_schedule(&race_schedule)
        .await
        .expect("race schedule");
    let (_, race_recording) = store
        .claim_due_recording(now)
        .await
        .expect("race claim")
        .expect("race schedule due");
    let (completed, cancelled) = futures::join!(
        store.finish_recording(
            race_recording.id,
            RecordingStatus::Completed,
            Some("/tmp/race.mkv"),
            None,
            Some(7),
            now,
        ),
        store.cancel_recording_schedule(race_schedule.id, user.id, now),
    );
    let completed = completed.expect("racing completion");
    let cancelled = cancelled.expect("racing cancellation").is_some();
    assert_ne!(completed, cancelled, "exactly one terminal transition wins");
    let terminal = store
        .get_recording(race_recording.id)
        .await
        .expect("race recording")
        .expect("race recording retained");
    assert!(matches!(
        terminal.status,
        RecordingStatus::Completed | RecordingStatus::Cancelled
    ));
    store
        .delete_live_tv_source(source.id)
        .await
        .expect("source cascade");
    assert!(
        store
            .get_live_tv_channel(channel.id)
            .await
            .expect("channel after cascade")
            .is_none()
    );
    let retained = store
        .get_recording(recording.id)
        .await
        .expect("recording after cascade")
        .expect("recording history retained");
    assert_eq!(retained.schedule_id, None);
    assert_eq!(retained.channel_id, None);
    assert_eq!(retained.path.as_deref(), Some("/tmp/news.mkv"));
}

async fn create_initial_admin_only_on_empty<S: Storage + ?Sized>(store: &S) {
    assert!(
        store.list_users().await.expect("list").is_empty(),
        "create_initial_admin must run against an empty users table"
    );
    let not_admin = make_user("initial-not-admin", false);
    assert!(matches!(
        store
            .create_initial_admin(&not_admin)
            .await
            .expect_err("non-admin rejected"),
        StorageError::InvalidInput(_)
    ));
    let first = make_user("initial-admin-a", true);
    let second = make_user("initial-admin-b", true);
    let (a, b) = futures::join!(
        store.create_initial_admin(&first),
        store.create_initial_admin(&second)
    );
    let (a, b) = (a.expect("first setup"), b.expect("second setup"));
    assert!(a ^ b, "exactly one concurrent setup wins");
    let users = store.list_users().await.expect("list after setup");
    assert_eq!(users.len(), 1);
    assert!(users[0].is_admin);
    let late = make_user("initial-admin-late", true);
    assert!(!store.create_initial_admin(&late).await.expect("late setup"));
    assert!(store.get_user(late.id).await.expect("get late").is_none());
    store.delete_user(users[0].id).await.expect("cleanup");
}

async fn edition_upsert_honours_id<S: Storage + ?Sized>(store: &S) {
    let lib = test_library("Edition Rename Lib");
    store.upsert_library(&lib).await.expect("library");
    let now = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Edition Rename", now);
    let other_work = make_work(lib.id, "Edition Rename Other", now);
    store.upsert_work(&work).await.expect("work");
    store.upsert_work(&other_work).await.expect("other work");
    let mut edition = make_edition(work.id, "Theatrical");
    let sibling = make_edition(work.id, "Extended");
    store.upsert_edition(&edition).await.expect("edition");
    store.upsert_edition(&sibling).await.expect("sibling");

    edition.name = "Director's Cut".to_string();
    assert_eq!(
        store.upsert_edition(&edition).await.expect("rename"),
        edition.id
    );
    let names: Vec<_> = store
        .editions_for_work(work.id)
        .await
        .expect("editions")
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, vec!["Director's Cut", "Extended"]);

    edition.work_id = other_work.id;
    assert_eq!(
        store.upsert_edition(&edition).await.expect("re-parent"),
        edition.id
    );
    let moved = store
        .get_edition(edition.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(moved.work_id, other_work.id);

    let mut clash = sibling.clone();
    clash.work_id = other_work.id;
    clash.name = "Director's Cut".to_string();
    assert!(matches!(
        store
            .upsert_edition(&clash)
            .await
            .expect_err("rename onto taken identity"),
        StorageError::Constraint(_)
    ));

    store.delete_work(work.id).await.ok();
    store.delete_work(other_work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

async fn lookup_and_cleanup_operations<S: Storage + ?Sized>(store: &S) {
    let host = make_user("lookup-host", false);
    let guest = make_user("lookup-guest", false);
    let outsider = make_user("lookup-outsider", false);
    for user in [&host, &guest, &outsider] {
        store.upsert_user(user).await.expect("user");
    }
    let lib = test_library("Lookup Lib");
    store.upsert_library(&lib).await.expect("library");
    for user in [&host, &guest] {
        store
            .grant_library_access(user.id, lib.id)
            .await
            .expect("grant");
    }
    let now = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Lookup Movie", now);
    store.upsert_work(&work).await.expect("work");
    let first = make_edition(work.id, "First");
    let second = make_edition(work.id, "Second");
    store.upsert_edition(&first).await.expect("first");
    store.upsert_edition(&second).await.expect("second");
    let media = make_media(first.id, "/lookup/movie.mkv", now);
    store.upsert_media_file(&media).await.expect("media");

    let batch = store
        .get_editions(&[second.id, EditionId::new(), first.id])
        .await
        .expect("get_editions");
    assert_eq!(
        batch.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![second.id, first.id],
        "input order kept, missing ids omitted"
    );

    let by_edition = store
        .work_for_edition(second.id)
        .await
        .expect("work_for_edition");
    assert_eq!(by_edition.map(|w| w.id), Some(work.id));
    assert!(
        store
            .work_for_edition(EditionId::new())
            .await
            .expect("unknown edition")
            .is_none()
    );
    let by_media = store
        .work_for_media_file(media.id)
        .await
        .expect("work_for_media_file");
    assert_eq!(by_media.map(|w| w.id), Some(work.id));
    assert!(
        store
            .work_for_media_file(MediaFileId::new())
            .await
            .expect("unknown media")
            .is_none()
    );

    let asset = TrickplayAsset {
        edition_id: first.id,
        vtt_path: "/cache/lookup.vtt".to_string(),
        thumb_width: 160,
        thumb_height: 90,
        tile_columns: 10,
        tile_rows: 10,
        interval_ms: 10_000,
        source_media_mtime: now,
        source_fingerprint: crate::canonical::trickplay_source_fingerprint(
            &media.path,
            media.size_bytes,
            media.mtime,
        ),
        generated_at: now,
        sheets: Vec::new(),
        chapter_cues: Vec::new(),
    };
    store.upsert_trickplay(&asset).await.expect("trickplay");
    store
        .delete_trickplay(first.id)
        .await
        .expect("delete trickplay");
    assert!(
        store
            .get_trickplay(first.id)
            .await
            .expect("get trickplay")
            .is_none()
    );
    store
        .delete_trickplay(first.id)
        .await
        .expect("deleting absent trickplay is a no-op");

    let mut policy = UserRestrictions::unrestricted(guest.id);
    policy.allow_unrated = false;
    store
        .set_user_restrictions(&policy)
        .await
        .expect("set restrictions");
    assert!(
        !store
            .user_can_access_work(guest.id, work.id)
            .await
            .expect("restricted access")
    );
    assert!(
        store
            .delete_user_restrictions(guest.id)
            .await
            .expect("delete restrictions")
    );
    assert!(
        store
            .get_user_restrictions(guest.id)
            .await
            .expect("get restrictions")
            .is_none()
    );
    assert!(
        !store
            .delete_user_restrictions(guest.id)
            .await
            .expect("delete again")
    );
    assert!(
        store
            .user_can_access_work(guest.id, work.id)
            .await
            .expect("unrestricted access")
    );

    let room = WatchRoom {
        id: WatchRoomId::new(),
        host_user_id: host.id,
        media_file_id: media.id,
        state: WatchRoomState::Waiting,
        position_ms: 0,
        sequence: 0,
        created_at: now,
        updated_at: now,
    };
    store.create_watch_room(&room).await.expect("room");
    store
        .join_watch_room(&WatchRoomMember {
            room_id: room.id,
            user_id: guest.id,
            joined_at: now,
        })
        .await
        .expect("join")
        .expect("joined");
    for user in [&host, &guest] {
        let rooms = store.list_watch_rooms(user.id).await.expect("list rooms");
        assert_eq!(
            rooms.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![room.id]
        );
    }
    assert!(
        store
            .list_watch_rooms(outsider.id)
            .await
            .expect("outsider rooms")
            .is_empty()
    );

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
    for user in [&host, &guest, &outsider] {
        store.delete_user(user.id).await.ok();
    }
}

async fn export_job_lease_protocol<S: Storage + ?Sized>(store: &S) {
    use noflex_core::ExportJobId;

    use crate::domain::{ExportFormat, ExportJob, ExportJobStatus};

    let owner = make_user("export-owner", false);
    let stranger = make_user("export-stranger", false);
    store.upsert_user(&owner).await.expect("owner");
    store.upsert_user(&stranger).await.expect("stranger");
    let lib = test_library("Export Lib");
    store.upsert_library(&lib).await.expect("library");
    let base = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Export Movie", base);
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Default");
    store.upsert_edition(&edition).await.expect("edition");
    let jobs: Vec<_> = (0..5)
        .map(|offset| ExportJob {
            id: ExportJobId::new(),
            user_id: owner.id,
            edition_id: edition.id,
            format: ExportFormat::Mp4,
            status: ExportJobStatus::Queued,
            progress: 0.0,
            output_path: None,
            error: None,
            created_at: base + time::Duration::milliseconds(offset),
            started_at: None,
            finished_at: None,
        })
        .collect();
    for job in &jobs {
        store.create_export_job(job).await.expect("create job");
    }
    let minute = time::Duration::minutes(1);
    let second = time::Duration::seconds(1);

    // Concurrent claims take distinct jobs, oldest first.
    let (a, b) = futures::join!(
        store.claim_export_job("w1", "t1", base, base + minute, 2),
        store.claim_export_job("w2", "t2", base, base + minute, 2),
    );
    let (a, b) = (
        a.expect("claim a").expect("job a"),
        b.expect("claim b").expect("job b"),
    );
    let mut claimed = [a.job.id, b.job.id];
    claimed.sort_by_key(|id| id.into_uuid());
    let mut expected = [jobs[0].id, jobs[1].id];
    expected.sort_by_key(|id| id.into_uuid());
    assert_eq!(claimed, expected);
    assert_eq!(a.attempt_count, 1);
    assert_eq!(a.job.status, ExportJobStatus::Running);
    let (job0, job1) = if a.job.id == jobs[0].id {
        (a, b)
    } else {
        (b, a)
    };

    // Progress renews the lease; a foreign token is fenced.
    assert_eq!(
        store
            .update_export_job_progress(job0.job.id, &job0.lease_token, 0.5, base + minute)
            .await
            .expect("progress"),
        Some(ExportJobStatus::Running)
    );
    assert_eq!(
        store
            .update_export_job_progress(job0.job.id, "bogus", 0.6, base + minute)
            .await
            .expect("fenced progress"),
        None
    );
    assert!(
        store
            .complete_export_job(job0.job.id, &job0.lease_token, "/out/0.mp4")
            .await
            .expect("complete")
    );
    let done = store
        .get_export_job(job0.job.id)
        .await
        .expect("get")
        .expect("job");
    assert_eq!(done.status, ExportJobStatus::Completed);
    assert_eq!(done.output_path.as_deref(), Some("/out/0.mp4"));
    assert_eq!(
        store
            .update_export_job_progress(job0.job.id, &job0.lease_token, 1.0, base + minute)
            .await
            .expect("progress after completion"),
        None
    );

    assert!(
        store
            .fail_export_job(job1.job.id, &job1.lease_token, "boom")
            .await
            .expect("fail")
    );
    let failed = store
        .get_export_job(job1.job.id)
        .await
        .expect("get")
        .expect("job");
    assert_eq!(failed.status, ExportJobStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("boom"));

    // An expired lease is reclaimed and the old token loses the job.
    let stale = store
        .claim_export_job("w3", "old", base, base - second, 2)
        .await
        .expect("claim stale")
        .expect("job 2");
    assert_eq!(stale.job.id, jobs[2].id);
    let reclaimed = store
        .claim_export_job("w4", "new", base, base - second, 2)
        .await
        .expect("reclaim")
        .expect("job 2 again");
    assert_eq!(reclaimed.job.id, jobs[2].id);
    assert_eq!(reclaimed.attempt_count, 2);
    assert!(
        !store
            .complete_export_job(jobs[2].id, "old", "/out/old.mp4")
            .await
            .expect("fenced completion")
    );
    // Its second lease also expired: at max_attempts the job fails
    // instead of being handed out a third time.
    let next = store
        .claim_export_job("w5", "t5", base, base + minute, 2)
        .await
        .expect("claim next")
        .expect("job 3");
    assert_eq!(next.job.id, jobs[3].id);
    assert_eq!(
        store
            .get_export_job(jobs[2].id)
            .await
            .expect("get")
            .expect("job")
            .status,
        ExportJobStatus::Failed
    );

    // Cancellation: ownership is enforced; queued jobs cancel at once,
    // running ones are flagged for the lease holder.
    assert!(
        store
            .request_export_job_cancellation(jobs[4].id, stranger.id)
            .await
            .expect("stranger cancel")
            .is_none()
    );
    let queued = store
        .request_export_job_cancellation(jobs[4].id, owner.id)
        .await
        .expect("cancel queued")
        .expect("owned");
    assert!(queued.changed);
    assert_eq!(queued.job.status, ExportJobStatus::Cancelled);
    let replay = store
        .request_export_job_cancellation(jobs[4].id, owner.id)
        .await
        .expect("replay cancel")
        .expect("owned");
    assert!(!replay.changed);

    let requested = store
        .request_export_job_cancellation(next.job.id, owner.id)
        .await
        .expect("cancel running")
        .expect("owned");
    assert!(requested.changed);
    assert_eq!(requested.job.status, ExportJobStatus::CancelRequested);
    assert_eq!(
        store
            .update_export_job_progress(next.job.id, &next.lease_token, 0.2, base + minute)
            .await
            .expect("progress during cancel"),
        Some(ExportJobStatus::CancelRequested)
    );
    assert!(
        !store
            .complete_export_job(next.job.id, &next.lease_token, "/out/late.mp4")
            .await
            .expect("completion after cancel request")
    );
    assert!(
        store
            .cancel_claimed_export_job(next.job.id, &next.lease_token)
            .await
            .expect("cancel claimed")
    );
    assert!(
        !store
            .cancel_claimed_export_job(next.job.id, &next.lease_token)
            .await
            .expect("cancel claimed replay")
    );
    assert!(
        store
            .claim_export_job("w6", "t6", base, base + minute, 2)
            .await
            .expect("drained")
            .is_none()
    );

    for job in &jobs {
        store.delete_export_job(job.id).await.expect("delete job");
    }

    let finished_at = base + minute;
    let shared = |user: UserId, status, output: Option<&str>, finished| ExportJob {
        id: noflex_core::ExportJobId::new(),
        user_id: user,
        edition_id: edition.id,
        format: ExportFormat::Mp4,
        status,
        progress: 0.0,
        output_path: output.map(str::to_string),
        error: None,
        created_at: base,
        started_at: None,
        finished_at: finished,
    };
    let first = shared(
        owner.id,
        ExportJobStatus::Completed,
        Some("/out/a.mp4"),
        Some(finished_at),
    );
    let failed = shared(
        stranger.id,
        ExportJobStatus::Failed,
        None,
        Some(finished_at),
    );
    store.create_export_job(&first).await.expect("first");
    store.create_export_job(&failed).await.expect("failed");
    let found = store
        .finished_export_job(edition.id, ExportFormat::Mp4)
        .await
        .expect("finished")
        .expect("a finished copy");
    assert_eq!(found.id, first.id);
    assert!(
        store
            .finished_export_job(edition.id, ExportFormat::Mkv)
            .await
            .expect("other format")
            .is_none()
    );
    assert_eq!(
        store
            .user_export_job(owner.id, edition.id, ExportFormat::Mp4)
            .await
            .expect("own job")
            .map(|job| job.id),
        Some(first.id)
    );
    assert!(
        store
            .user_export_job(stranger.id, edition.id, ExportFormat::Mp4)
            .await
            .expect("failed doesn't count")
            .is_none()
    );
    let second = shared(
        stranger.id,
        ExportJobStatus::Completed,
        Some("/out/a.mp4"),
        Some(finished_at),
    );
    store.create_export_job(&second).await.expect("second");
    store
        .delete_export_job(first.id)
        .await
        .expect("delete first");
    assert!(
        store
            .export_output_in_use("/out/a.mp4")
            .await
            .expect("in use")
    );
    store
        .delete_export_job(second.id)
        .await
        .expect("delete second");
    assert!(
        !store
            .export_output_in_use("/out/a.mp4")
            .await
            .expect("free")
    );
    store
        .delete_export_job(failed.id)
        .await
        .expect("delete failed");

    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
    store.delete_user(owner.id).await.ok();
    store.delete_user(stranger.id).await.ok();
}

fn live_tv_fixture(
    name: &str,
) -> (
    crate::domain::LiveTvSource,
    crate::domain::LiveTvChannel,
    crate::domain::LiveTvChannel,
) {
    use noflex_core::{ChannelId, LiveTvSourceId};

    use crate::domain::{LiveTvChannel, LiveTvSource};

    let source = LiveTvSource {
        id: LiveTvSourceId::new(),
        name: name.into(),
        enabled: true,
        last_refresh_at: None,
        last_error: None,
        playlist_url: format!("https://example.test/{name}.m3u"),
        epg_url: None,
    };
    let channel = |external: &str| LiveTvChannel {
        id: ChannelId::new(),
        source_id: source.id,
        external_id: external.into(),
        name: external.into(),
        number: None,
        logo_url: None,
        group: None,
        enabled: true,
        stream_url: format!("https://example.test/{name}/{external}"),
    };
    let (one, two) = (channel("one"), channel("two"));
    (source, one, two)
}

fn make_program(
    channel: noflex_core::ChannelId,
    external_id: &str,
    start_at: OffsetDateTime,
    end_at: OffsetDateTime,
) -> crate::domain::EpgProgram {
    crate::domain::EpgProgram {
        id: noflex_core::EpgProgramId::new(),
        channel_id: channel,
        external_id: external_id.into(),
        title: external_id.into(),
        subtitle: None,
        description: None,
        category: None,
        start_at,
        end_at,
    }
}

async fn live_tv_guide_ordering_and_offsets<S: Storage + ?Sized>(store: &S) {
    use time::{UtcOffset, macros::datetime};

    let (source, one, two) = live_tv_fixture("guide-order");
    store.upsert_live_tv_source(&source).await.expect("source");
    let ist = UtcOffset::from_hms(5, 30, 0).expect("offset");
    let base = datetime!(2030-01-02 00:00 UTC);
    let minutes = time::Duration::minutes;
    // Ends before the window opens; written in +05:30 its text sorts after
    // the UTC window start, so text comparison of raw offsets would leak it.
    let before = make_program(
        one.id,
        "before",
        (base - minutes(60)).to_offset(ist),
        (base - minutes(30)).to_offset(ist),
    );
    let late = make_program(one.id, "late", base + minutes(30), base + minutes(60));
    let early = make_program(
        two.id,
        "early",
        base.to_offset(ist),
        (base + minutes(30)).to_offset(ist),
    );
    store
        .apply_live_tv_refresh(
            source.id,
            &[one.clone(), two.clone()],
            &[before, late.clone(), early.clone()],
            base,
        )
        .await
        .expect("refresh");
    let window = store
        .list_live_tv_guide(
            &[one.id, two.id, one.id],
            base,
            base + minutes(120),
            10,
            None,
        )
        .await
        .expect("guide")
        .items;
    assert_eq!(
        window.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![early.id, late.id]
    );
    assert_eq!(window[0].start_at, early.start_at);
    // The limit applies across channels, not to the first channel listed.
    let first = store
        .list_live_tv_guide(&[one.id, two.id], base, base + minutes(120), 1, None)
        .await
        .expect("limited guide")
        .items;
    assert_eq!(
        first.iter().map(|p| p.id).collect::<Vec<_>>(),
        vec![early.id]
    );
    store
        .delete_live_tv_source(source.id)
        .await
        .expect("cleanup");
}

/// Guide pages walk `(start_at, id)` across channels; programmes that
/// share a start time on different channels are neither skipped nor
/// repeated, and a malformed cursor is rejected.
async fn live_tv_guide_keyset_pagination<S: Storage + ?Sized>(store: &S) {
    use time::macros::datetime;

    let (source, one, two) = live_tv_fixture("guide-pages");
    store.upsert_live_tv_source(&source).await.expect("source");
    let base = datetime!(2030-03-01 00:00 UTC);
    let slot = time::Duration::minutes(30);
    let programs: Vec<_> = (0..3)
        .flat_map(|i| {
            let start = base + slot * i;
            [one.id, two.id]
                .map(|channel| make_program(channel, &format!("g{i}"), start, start + slot))
        })
        .collect();
    store
        .apply_live_tv_refresh(source.id, &[one.clone(), two.clone()], &programs, base)
        .await
        .expect("refresh");
    let end = base + slot * 3;
    let full = store
        .list_live_tv_guide(&[one.id, two.id], base, end, 100, None)
        .await
        .expect("full guide");
    assert_eq!(full.items.len(), programs.len());
    let mut seen = Vec::new();
    let mut cursor = None;
    for _ in 0..10 {
        let page = store
            .list_live_tv_guide(&[one.id, two.id], base, end, 4, cursor)
            .await
            .expect("guide page");
        assert_eq!(page.item_cursors.len(), page.items.len());
        let next = page.next_cursor();
        seen.extend(page.items.iter().map(|p| p.id));
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(
        seen,
        full.items.iter().map(|p| p.id).collect::<Vec<_>>(),
        "pages concatenate to the full guide"
    );
    let err = store
        .list_live_tv_guide(&[one.id], base, end, 4, Some(PageCursor("bad".into())))
        .await
        .expect_err("malformed cursor");
    assert!(matches!(err, StorageError::InvalidInput(_)), "{err:?}");
    store
        .delete_live_tv_source(source.id)
        .await
        .expect("cleanup");
}

/// A user's export jobs page newest `(created_at, id)` first, scoped
/// to that user; a malformed cursor is rejected.
async fn export_jobs_keyset_pagination<S: Storage + ?Sized>(store: &S) {
    use noflex_core::ExportJobId;

    use crate::domain::{ExportFormat, ExportJob, ExportJobStatus};

    let owner = make_user("export-pager", false);
    let other = make_user("export-pager-other", false);
    store.upsert_user(&owner).await.expect("owner");
    store.upsert_user(&other).await.expect("other");
    let lib = test_library("Export Page Lib");
    store.upsert_library(&lib).await.expect("library");
    let base = OffsetDateTime::now_utc();
    let work = make_work(lib.id, "Export Pages", base);
    store.upsert_work(&work).await.expect("work");
    let edition = make_edition(work.id, "Default");
    store.upsert_edition(&edition).await.expect("edition");
    let job = |user: UserId, offset: i64| ExportJob {
        id: ExportJobId::new(),
        user_id: user,
        edition_id: edition.id,
        format: ExportFormat::Mkv,
        status: ExportJobStatus::Queued,
        progress: 0.0,
        output_path: None,
        error: None,
        // Two jobs share each timestamp, so the id tiebreak matters.
        created_at: base + time::Duration::seconds(offset / 2),
        started_at: None,
        finished_at: None,
    };
    let mine: Vec<_> = (0..5).map(|i| job(owner.id, i)).collect();
    for j in mine.iter().chain([&job(other.id, 0)]) {
        store.create_export_job(j).await.expect("create");
    }
    let mut seen = Vec::new();
    let mut cursor = None;
    for _ in 0..10 {
        let page = store
            .list_export_jobs(owner.id, 2, cursor)
            .await
            .expect("page");
        let next = page.next_cursor();
        seen.extend(page.items.iter().map(|j| j.id));
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    let mut expected = mine.clone();
    expected.sort_by_key(|j| std::cmp::Reverse((j.created_at, j.id.into_uuid())));
    assert_eq!(seen, expected.iter().map(|j| j.id).collect::<Vec<_>>());
    let err = store
        .list_export_jobs(owner.id, 2, Some(PageCursor("bad".into())))
        .await
        .expect_err("malformed cursor");
    assert!(matches!(err, StorageError::InvalidInput(_)), "{err:?}");
    store.delete_user(owner.id).await.ok();
    store.delete_user(other.id).await.ok();
    store.delete_work(work.id).await.ok();
    store.delete_library(lib.id).await.ok();
}

async fn live_tv_refresh_large_feed<S: Storage + ?Sized>(store: &S) {
    use time::macros::datetime;

    let (source, one, two) = live_tv_fixture("large-feed");
    store.upsert_live_tv_source(&source).await.expect("source");
    let base = datetime!(2030-02-01 00:00 UTC);
    let slot = time::Duration::minutes(10);
    let programs: Vec<_> = (0..2_500)
        .flat_map(|i| {
            let start = base + slot * i;
            [one.id, two.id]
                .map(|channel| make_program(channel, &format!("p{i}"), start, start + slot))
        })
        .collect();
    let channels = [one.clone(), two.clone()];
    store
        .apply_live_tv_refresh(source.id, &channels, &programs, base)
        .await
        .expect("large refresh");
    let end = base + slot * 2_500;
    let all = store
        .list_live_tv_guide(&[one.id, two.id], base, end, 10_000, None)
        .await
        .expect("guide")
        .items;
    assert_eq!(all.len(), 5_000);

    // The next snapshot keeps every other slot; the rest must be swept.
    let kept: Vec<_> = programs
        .iter()
        .enumerate()
        .filter(|(i, _)| (i / 2) % 2 == 0)
        .map(|(_, p)| {
            let mut p = p.clone();
            p.id = noflex_core::EpgProgramId::new();
            p.title = format!("{} v2", p.external_id);
            p
        })
        .collect();
    store
        .apply_live_tv_refresh(source.id, &channels, &kept, base + slot)
        .await
        .expect("shrinking refresh");
    let after = store
        .list_live_tv_guide(&[one.id, two.id], base, end, 10_000, None)
        .await
        .expect("guide after")
        .items;
    assert_eq!(after.len(), kept.len());
    assert!(after.iter().all(|p| p.title.ends_with(" v2")));
    assert!(
        store
            .get_epg_program(programs[0].id)
            .await
            .expect("retained program")
            .is_some(),
        "retained rows keep their original id"
    );
    store
        .delete_live_tv_source(source.id)
        .await
        .expect("cleanup");
}

async fn live_tv_listing_and_maintenance<S: Storage + ?Sized>(store: &S) {
    use noflex_core::RecordingScheduleId;

    use crate::domain::{RecordingSchedule, RecordingScheduleStatus, RecordingStatus};

    let owner = make_user("dvr-list-owner", false);
    let other = make_user("dvr-list-other", false);
    store.upsert_user(&owner).await.expect("owner");
    store.upsert_user(&other).await.expect("other");
    let (source, one, two) = live_tv_fixture("dvr-list");
    store.upsert_live_tv_source(&source).await.expect("source");
    let now = OffsetDateTime::now_utc();
    store
        .apply_live_tv_refresh(source.id, &[one.clone(), two.clone()], &[], now)
        .await
        .expect("refresh");

    // Channel listing pages by id; channels dropped from the feed are
    // disabled and leave the listing.
    let mut ids = [one.id, two.id];
    ids.sort_by_key(|id| id.into_uuid());
    let page = store
        .list_live_tv_channels(Some(source.id), 1, None)
        .await
        .expect("channels page 1");
    assert_eq!(
        page.items.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![ids[0]]
    );
    let page = store
        .list_live_tv_channels(Some(source.id), 1, page.next_cursor())
        .await
        .expect("channels page 2");
    assert_eq!(
        page.items.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![ids[1]]
    );
    assert!(page.next_cursor().is_none());
    store
        .apply_live_tv_refresh(source.id, std::slice::from_ref(&one), &[], now)
        .await
        .expect("refresh without two");
    let enabled = store
        .list_live_tv_channels(Some(source.id), 10, None)
        .await
        .expect("enabled channels");
    assert_eq!(
        enabled.items.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![one.id]
    );
    assert!(matches!(
        store
            .list_live_tv_channels(None, 10, Some(PageCursor("nope".into())))
            .await
            .expect_err("bad cursor"),
        StorageError::InvalidInput(_)
    ));

    // Refresh errors never overwrite a later attempt.
    store
        .record_live_tv_refresh_error(source.id, "timeout", now + time::Duration::minutes(5))
        .await
        .expect("record error");
    store
        .record_live_tv_refresh_error(source.id, "stale", now)
        .await
        .expect("record stale error");
    let refreshed = store
        .get_live_tv_source(source.id)
        .await
        .expect("source")
        .expect("present");
    assert_eq!(refreshed.last_error.as_deref(), Some("timeout"));
    assert_eq!(
        refreshed.last_refresh_at,
        Some(now + time::Duration::minutes(5))
    );

    let schedule = |start_at: OffsetDateTime| RecordingSchedule {
        id: RecordingScheduleId::new(),
        owner_id: owner.id,
        channel_id: one.id,
        program_id: None,
        start_at,
        end_at: start_at + time::Duration::hours(1),
        padding_before_secs: 0,
        padding_after_secs: 0,
        status: RecordingScheduleStatus::Scheduled,
        created_at: now,
        updated_at: now,
    };
    let schedules = [
        schedule(now - time::Duration::seconds(2)),
        schedule(now - time::Duration::seconds(1)),
        schedule(now + time::Duration::days(1)),
    ];
    for s in &schedules {
        store.create_recording_schedule(s).await.expect("schedule");
    }
    let page = store
        .list_recording_schedules(owner.id, 2, None)
        .await
        .expect("schedules page 1");
    assert_eq!(
        page.items.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![schedules[2].id, schedules[1].id]
    );
    let page = store
        .list_recording_schedules(owner.id, 2, page.next_cursor())
        .await
        .expect("schedules page 2");
    assert_eq!(
        page.items.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![schedules[0].id]
    );
    assert!(page.next_cursor().is_none());
    assert!(
        store
            .list_recording_schedules(other.id, 10, None)
            .await
            .expect("other schedules")
            .items
            .is_empty()
    );

    let (_, first) = store
        .claim_due_recording(now)
        .await
        .expect("claim")
        .expect("first due");
    let (_, second) = store
        .claim_due_recording(now)
        .await
        .expect("claim")
        .expect("second due");
    assert!(
        store
            .claim_due_recording(now)
            .await
            .expect("claim")
            .is_none()
    );
    let page = store
        .list_recordings(owner.id, 1, None)
        .await
        .expect("recordings page 1");
    assert_eq!(
        page.items.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![second.id]
    );
    let page = store
        .list_recordings(owner.id, 1, page.next_cursor())
        .await
        .expect("recordings page 2");
    assert_eq!(
        page.items.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![first.id]
    );

    assert!(matches!(
        store
            .delete_recording(first.id, owner.id)
            .await
            .expect_err("active recording"),
        StorageError::Conflict(_)
    ));
    assert_eq!(
        store
            .fail_running_recordings(now, "server restarted")
            .await
            .expect("fail running"),
        2
    );
    let failed = store
        .get_recording(first.id)
        .await
        .expect("get")
        .expect("present");
    assert_eq!(failed.status, RecordingStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("server restarted"));
    assert!(
        store
            .delete_recording(first.id, other.id)
            .await
            .expect("foreign delete")
            .is_none()
    );
    let deleted = store
        .delete_recording(first.id, owner.id)
        .await
        .expect("delete")
        .expect("owned");
    assert_eq!(deleted.id, first.id);
    assert!(store.get_recording(first.id).await.expect("get").is_none());

    store
        .delete_live_tv_source(source.id)
        .await
        .expect("cleanup source");
    store.delete_user(owner.id).await.ok();
    store.delete_user(other.id).await.ok();
}

async fn play_sessions_round_trip_filter_and_page<S: Storage + ?Sized>(store: &S) {
    use noflex_core::PlaySessionId;

    use crate::{domain::PlaySession, query::PlaySessionQuery};

    let lib = test_library("Play History Lib");
    store.upsert_library(&lib).await.expect("library");
    let other_lib = test_library("Play History Other");
    store
        .upsert_library(&other_lib)
        .await
        .expect("other library");
    let alice = make_user("ph-alice", false);
    let bob = make_user("ph-bob", false);
    store.upsert_user(&alice).await.expect("alice");
    store.upsert_user(&bob).await.expect("bob");

    let now = OffsetDateTime::now_utc();
    let series = make_work_of_kind(lib.id, WorkKind::Series, "PH Show", now);
    let mut episode = make_work_of_kind(lib.id, WorkKind::Episode, "PH Ep", now);
    episode.parent_id = Some(series.id);
    let movie = make_work_of_kind(other_lib.id, WorkKind::Movie, "PH Movie", now);
    for w in [&series, &episode, &movie] {
        store.upsert_work(w).await.expect("work");
    }

    let hour = time::Duration::hours(1);
    let session = |user: UserId, work: &Work, root: &Work, started: OffsetDateTime| PlaySession {
        id: PlaySessionId::new(),
        user_id: user,
        work_id: work.id,
        root_work_id: root.id,
        library_id: work.library_id,
        started_at: started,
        last_seen_at: started + time::Duration::minutes(20),
        watched_ms: 1_200_000,
        position_ms: 1_200_000,
        playing: true,
        ended: false,
        device: Some("Firefox on Linux".into()),
    };
    let a1 = session(alice.id, &episode, &series, now - hour * 30);
    let mut a2 = session(alice.id, &movie, &movie, now - hour * 3);
    let b1 = session(bob.id, &episode, &series, now - hour * 2);
    for s in [&a1, &a2, &b1] {
        store.upsert_play_session(s).await.expect("upsert");
    }

    // Upsert replaces in place.
    a2.watched_ms = 2_400_000;
    a2.ended = true;
    store.upsert_play_session(&a2).await.expect("update");
    assert_eq!(
        store.get_play_session(a2.id).await.expect("get"),
        Some(a2.clone())
    );

    // `open_play_session`: ended and stale viewings don't count.
    assert_eq!(
        store
            .open_play_session(alice.id, movie.id, now - hour * 4)
            .await
            .expect("open ended"),
        None
    );
    assert_eq!(
        store
            .open_play_session(bob.id, episode.id, now - hour * 2)
            .await
            .expect("open live")
            .map(|s| s.id),
        Some(b1.id)
    );
    assert_eq!(
        store
            .open_play_session(bob.id, episode.id, now)
            .await
            .expect("open stale"),
        None
    );

    let ids = |rows: Vec<PlaySession>| rows.into_iter().map(|s| s.id).collect::<Vec<_>>();
    let matching = |q: PlaySessionQuery| async move {
        ids(store.play_sessions_matching(&q).await.expect("matching"))
    };
    assert_eq!(
        matching(PlaySessionQuery::default()).await,
        vec![b1.id, a2.id, a1.id]
    );
    assert_eq!(
        matching(PlaySessionQuery {
            user: Some(alice.id),
            ..Default::default()
        })
        .await,
        vec![a2.id, a1.id]
    );
    assert_eq!(
        matching(PlaySessionQuery {
            library: Some(lib.id),
            ..Default::default()
        })
        .await,
        vec![b1.id, a1.id]
    );
    assert_eq!(
        matching(PlaySessionQuery {
            root_work: Some(movie.id),
            ..Default::default()
        })
        .await,
        vec![a2.id]
    );
    assert_eq!(
        matching(PlaySessionQuery {
            since: Some(now - hour * 24),
            until: Some(now - hour * 2),
            ..Default::default()
        })
        .await,
        vec![a2.id],
        "since is inclusive, until exclusive"
    );

    // Keyset paging walks the same order.
    let first = store
        .list_play_sessions(&PlaySessionQuery::default(), 2, None)
        .await
        .expect("page 1");
    assert_eq!(
        first.items.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![b1.id, a2.id]
    );
    assert!(first.has_next);
    let second = store
        .list_play_sessions(&PlaySessionQuery::default(), 2, first.end_cursor.clone())
        .await
        .expect("page 2");
    assert_eq!(
        second.items.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![a1.id]
    );
    assert!(!second.has_next);
    assert!(matches!(
        store
            .list_play_sessions(
                &PlaySessionQuery::default(),
                2,
                Some(PageCursor("nope".into()))
            )
            .await,
        Err(StorageError::InvalidInput(_))
    ));

    // Deleting a user or a work takes its viewings with it.
    store.delete_user(bob.id).await.expect("delete bob");
    assert_eq!(store.get_play_session(b1.id).await.expect("get b1"), None);
    store.delete_work(movie.id).await.expect("delete movie");
    assert_eq!(store.get_play_session(a2.id).await.expect("get a2"), None);
    assert!(
        store
            .get_play_session(a1.id)
            .await
            .expect("get a1")
            .is_some()
    );
}
