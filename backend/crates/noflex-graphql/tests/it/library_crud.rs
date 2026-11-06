// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::Principal;
use noflex_core::UserId;
use noflex_graphql::{NoflexSchema, SchemaContext, build_schema};
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

async fn schema() -> (NoflexSchema, User, User) {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let admin = user("admin", true);
    let member = user("member", false);
    store.upsert_user(&admin).await.expect("admin");
    store.upsert_user(&member).await.expect("member");
    (
        build_schema(SchemaContext::new(Arc::new(store))),
        admin,
        member,
    )
}

fn user(name: &str, is_admin: bool) -> User {
    User {
        id: UserId::new(),
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

#[tokio::test]
async fn admin_library_crud_and_config_round_trip() {
    let (schema, admin, _) = schema().await;
    let create = r#"mutation { createLibrary(name: "Films", kind: MOVIES, paths: ["/srv/films"]) { id name kind paths } }"#;
    let response = schema.execute(request(create, &admin)).await;
    assert!(response.errors.is_empty(), "create: {:?}", response.errors);
    let data = response.data.into_json().unwrap();
    let id = data["createLibrary"]["id"].as_str().unwrap();
    assert_eq!(data["createLibrary"]["name"], "Films");

    let update = format!(
        r#"mutation {{ updateLibrary(id: "{id}", name: "Films HD", kind: SHOWS) {{ name kind }} }}"#
    );
    let response = schema.execute(request(update, &admin)).await;
    assert!(response.errors.is_empty(), "update: {:?}", response.errors);
    assert_eq!(
        response.data.into_json().unwrap()["updateLibrary"]["kind"],
        "SHOWS"
    );

    let config = format!(
        r#"mutation {{ updateLibraryConfig(id: "{id}", preferredLanguages: ["en", "ja"], preferredProviders: ["tmdb"], excludedFolders: ["*/.staging"], watchEnabled: false, scanSchedule: DAILY) {{ config {{ preferredLanguages preferredProviders excludedFolders watchEnabled scanSchedule }} }} }}"#
    );
    let response = schema.execute(request(config, &admin)).await;
    assert!(response.errors.is_empty(), "config: {:?}", response.errors);
    let config = &response.data.into_json().unwrap()["updateLibraryConfig"]["config"];
    assert_eq!(
        config["preferredLanguages"],
        serde_json::json!(["en", "ja"])
    );
    assert_eq!(config["watchEnabled"], false);
    assert_eq!(config["scanSchedule"], "DAILY");

    let delete = format!(r#"mutation {{ deleteLibrary(id: "{id}") {{ id }} }}"#);
    let response = schema.execute(request(delete, &admin)).await;
    assert!(response.errors.is_empty(), "delete: {:?}", response.errors);
    assert!(response.data.into_json().unwrap()["deleteLibrary"].is_object());
}

#[tokio::test]
async fn anonymous_and_non_admin_library_administration_is_denied() {
    let (schema, admin, member) = schema().await;
    let create =
        r#"mutation { createLibrary(name: "Films", kind: MOVIES, paths: ["/srv/films"]) { id } }"#;
    let response = schema.execute(create).await;
    assert_eq!(response.errors[0].message, "authentication required");
    let response = schema.execute(request(create, &member)).await;
    assert_eq!(response.errors[0].message, "administrator required");

    let response = schema.execute(request(create, &admin)).await;
    let id = response.data.into_json().unwrap()["createLibrary"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    for mutation in [
        format!(r#"mutation {{ updateLibrary(id: "{id}", name: "No") {{ id }} }}"#),
        format!(r#"mutation {{ updateLibraryConfig(id: "{id}", watchEnabled: false) {{ id }} }}"#),
        format!(r#"mutation {{ deleteLibrary(id: "{id}") {{ id }} }}"#),
    ] {
        let response = schema.execute(mutation.clone()).await;
        assert_eq!(response.errors[0].message, "authentication required");
        let response = schema.execute(request(mutation, &member)).await;
        assert_eq!(response.errors[0].message, "administrator required");
    }
}

#[tokio::test]
async fn stale_admin_claim_is_denied_after_demotion() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let mut former_admin = user("former-admin", true);
    store.upsert_user(&former_admin).await.expect("admin");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let schema = build_schema(SchemaContext::new(storage.clone()));
    former_admin.is_admin = false;
    former_admin.role = noflex_core::UserRole::User;
    storage.upsert_user(&former_admin).await.expect("demote");

    // The admin gate trusts the request's resolved role, so the stale
    // claim has to die where the transport resolves it: the token still
    // says admin, but resolution reads the demoted row.
    let jwt = noflex_auth::JwtConfig::dev(b"library-crud-test-secret-32-bytes").expect("jwt");
    let now = time::OffsetDateTime::now_utc();
    let session = noflex_storage::domain::Session {
        id: noflex_core::SessionId::new(),
        user_id: former_admin.id,
        client: None,
        device: None,
        device_id: None,
        app_version: None,
        remote_addr: None,
        created_at: now,
        last_seen_at: now,
        expires_at: now + time::Duration::days(1),
        refresh_token_hash: None,
    };
    storage.create_session(&session).await.expect("session");
    let token = noflex_auth::issue_jwt(&jwt, former_admin.id, true, session.id).expect("token");
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {token}").parse().expect("header"),
    );
    let resolved = noflex_auth::principal_from_headers(&headers, &jwt, storage.as_ref()).await;
    assert!(resolved.is_authenticated());
    assert!(!resolved.is_admin, "role comes from storage, not the claim");
    let request = Request::new(
        r#"mutation { createLibrary(name: "Films", kind: MOVIES, paths: ["/srv/films"]) { id } }"#,
    )
    .data(resolved)
    .data(RequestUser {
        user_id: resolved.user_id,
        is_admin: resolved.is_admin,
    });
    let response = schema.execute(request).await;
    assert_eq!(response.errors[0].message, "administrator required");
}

#[tokio::test]
async fn plugin_override_requires_admin_and_a_registered_plugin_hook() {
    use noflex_plugin_host::{EngineConfig, PluginRegistry, PriorityTable, build_engine};

    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let admin = user("admin", true);
    let member = user("member", false);
    store.upsert_user(&admin).await.expect("admin");
    store.upsert_user(&member).await.expect("member");
    let registry = Arc::new(
        PluginRegistry::new(build_engine(&EngineConfig::default()).expect("engine"))
            .expect("registry"),
    );
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store)),
        plugins: Some(registry),
        priority_table: Some(Arc::new(tokio::sync::RwLock::new(PriorityTable::default()))),
        ..Default::default()
    });
    let mutation = r#"mutation { setPluginOverride(pluginId: "missing", hook: METADATA, workKind: MOVIE, priority: 200) { pluginId } }"#;

    for operation in [
        "{ plugins { id } }",
        "{ pluginOverrides { pluginId } }",
        mutation,
    ] {
        let response = schema.execute(operation).await;
        assert_eq!(response.errors[0].message, "authentication required");
        let response = schema.execute(request(operation, &member)).await;
        assert_eq!(response.errors[0].message, "administrator required");
    }
    let response = schema.execute(request(mutation, &admin)).await;
    assert_eq!(response.errors[0].message, "plugin is not loaded");
}
