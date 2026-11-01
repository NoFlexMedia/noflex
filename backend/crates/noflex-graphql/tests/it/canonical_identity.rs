// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_core::UserId;
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::{RequestUser, domain::ActivityKind, store::Storage};
use noflex_storage_turso::TursoStore;

fn as_admin(query: &str) -> Request {
    Request::new(query).data(RequestUser {
        user_id: UserId::new(),
        is_admin: true,
    })
}

#[tokio::test]
async fn activity_requires_authentication() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let schema = build_schema(SchemaContext::new(Arc::new(store)));
    let resp = schema.execute("{ activity { nodes { id } } }").await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "{:?}",
        resp.errors
    );
}

#[tokio::test]
async fn activity_kind_round_trips_as_typed_enum_on_gql_wire() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let schema = build_schema(SchemaContext::new(Arc::new(store)));

    let now = time::OffsetDateTime::now_utc();
    let id = schema
        .data::<Arc<dyn Storage>>()
        .expect("storage wired")
        .append_activity(
            ActivityKind::ScanCompleted,
            Some("scanner"),
            None,
            None,
            now,
        )
        .await
        .expect("append");
    assert!(id > 0);

    let query = r#"{ activity(first: 10) { nodes { id kind actor subject detail } } }"#;
    let resp = schema.execute(as_admin(query)).await;
    assert!(resp.errors.is_empty(), "activity query: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let rows = data["activity"]["nodes"].as_array().expect("rows array");
    assert_eq!(rows.len(), 1, "one row written, one row read");
    let row = &rows[0];
    assert_eq!(row["kind"], "SCAN_COMPLETED", "typed enum wire form");
    assert_eq!(row["actor"], "scanner");
    assert!(row["subject"].is_null());
    assert!(row["detail"].is_null());
}

#[tokio::test]
async fn activity_kind_filter_passes_through_typed_enum() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let schema = build_schema(SchemaContext::new(Arc::new(store)));

    let storage = schema
        .data::<Arc<dyn Storage>>()
        .expect("storage wired")
        .clone();
    let now = time::OffsetDateTime::now_utc();
    storage
        .append_activity(ActivityKind::ScanCompleted, Some("a"), None, None, now)
        .await
        .expect("scan append");
    storage
        .append_activity(ActivityKind::ItemAdded, Some("b"), None, None, now)
        .await
        .expect("item append");

    let query = r#"{ activity(kinds: [SCAN_COMPLETED], first: 10) { nodes { kind } } }"#;
    let resp = schema.execute(as_admin(query)).await;
    assert!(resp.errors.is_empty(), "filter query: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let rows = data["activity"]["nodes"].as_array().expect("rows array");
    assert_eq!(rows.len(), 1, "filter narrows to one row");
    assert_eq!(rows[0]["kind"], "SCAN_COMPLETED");
}
