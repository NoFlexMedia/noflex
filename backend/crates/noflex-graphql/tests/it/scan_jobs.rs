// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::Principal;
use noflex_graphql::{SchemaContext, build_schema};
use noflex_scanner::{FilesystemScanner, ScanRegistry};
use noflex_storage::{RequestUser, Storage, domain::User};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

fn request(query: impl Into<String>, user: &User) -> Request {
    let principal = Principal {
        user_id: user.id,
        is_admin: user.is_admin,
        session_id: None,
    };
    Request::new(query).data(principal).data(RequestUser {
        user_id: user.id,
        is_admin: user.is_admin,
    })
}

fn user(name: &str, is_admin: bool) -> User {
    User {
        id: noflex_core::UserId::new(),
        username: name.into(),
        display_name: None,
        password_hash: "argon2id$placeholder".into(),
        is_admin,
        role: if is_admin {
            noflex_core::UserRole::Admin
        } else {
            noflex_core::UserRole::User
        },
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn admin_can_trigger_and_inspect_a_scan() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let admin = user("admin", true);
    store.upsert_user(&admin).await.expect("admin");
    let library = noflex_storage::domain::Library {
        id: noflex_core::LibraryId::new(),
        name: "Films".into(),
        kind: noflex_storage::domain::LibraryKind::Movies,
        paths: vec!["/tmp/noflex-empty".into()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("library");

    let scanner_store = Arc::new(TursoStore::open(":memory:").await.expect("scan store"));
    scanner_store.migrate().await.expect("migrate scanner");
    scanner_store
        .upsert_library(&library)
        .await
        .expect("scanner library");
    let registry = Arc::new(ScanRegistry::new(Arc::new(FilesystemScanner::new(
        scanner_store,
    ))));
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store)),
        scanner: Some(registry),
        ..Default::default()
    });

    let trigger = format!(
        r#"mutation {{ triggerLibraryScan(id: "{}") {{ state startedAt }} }}"#,
        library.id
    );
    let response = schema.execute(request(trigger, &admin)).await;
    assert!(response.errors.is_empty(), "trigger: {:?}", response.errors);
    assert_eq!(
        response.data.into_json().unwrap()["triggerLibraryScan"]["state"],
        "RUNNING"
    );

    for _ in 0..50 {
        let status = format!(
            r#"{{ libraryScanStatus(id: "{}") {{ state finishedAt }} }}"#,
            library.id
        );
        let response = schema.execute(request(status, &admin)).await;
        assert!(response.errors.is_empty(), "status: {:?}", response.errors);
        let data = response.data.into_json().unwrap();
        if data["libraryScanStatus"]["state"] != "RUNNING" {
            assert_eq!(data["libraryScanStatus"]["state"], "COMPLETED");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("scan never finished");
}

#[tokio::test]
async fn anonymous_and_non_admin_scan_administration_is_denied() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let member = user("member", false);
    store.upsert_user(&member).await.expect("member");
    let registry = Arc::new(ScanRegistry::new(Arc::new(FilesystemScanner::new(
        Arc::new(TursoStore::open(":memory:").await.expect("scan store")),
    ))));
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store)),
        scanner: Some(registry),
        ..Default::default()
    });
    let id = noflex_core::LibraryId::new();
    for operation in [
        format!(r#"mutation {{ triggerLibraryScan(id: "{id}") {{ state }} }}"#),
        format!(r#"{{ libraryScanStatus(id: "{id}") {{ state }} }}"#),
    ] {
        let response = schema.execute(operation.clone()).await;
        assert_eq!(response.errors[0].message, "authentication required");
        let response = schema.execute(request(operation, &member)).await;
        assert_eq!(response.errors[0].message, "administrator required");
    }
}
