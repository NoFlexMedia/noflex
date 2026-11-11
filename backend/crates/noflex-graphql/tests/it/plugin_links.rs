// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{SessionId, UserId};
use noflex_graphql::{PluginLinkChallenge, PluginLinkService, SchemaContext, build_schema};
use noflex_plugin_host::{EngineConfig, PluginRegistry, build_engine};
use noflex_storage::{RequestUser, store::Storage};
use noflex_storage_turso::TursoStore;

fn as_request_user(principal: Principal) -> RequestUser {
    RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    }
}

const TEST_JWT_SECRET: &[u8] = b"test-secret-32-bytes-please-use-it";

async fn authed_schema() -> (
    noflex_graphql::NoflexSchema,
    Arc<JwtConfig>,
    Arc<TursoStore>,
) {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(PluginRegistry::new(engine).expect("registry"));
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        plugins: Some(registry),
        jwt: Some(jwt.clone()),
        plugin_links: Some(Arc::new(StubLinkService)),
        ..Default::default()
    });
    (schema, jwt, store)
}

async fn setup_admin(schema: &noflex_graphql::NoflexSchema, jwt: &JwtConfig) -> Principal {
    let resp = schema
        .execute(
            r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId } }"#,
        )
        .await;
    assert!(resp.errors.is_empty(), "wizard: {:?}", resp.errors);
    let token = login(schema, "admin", "hunter2").await;
    principal_from_token(jwt, &token)
}

async fn login(schema: &noflex_graphql::NoflexSchema, username: &str, password: &str) -> String {
    let q = format!(
        r#"mutation {{ login(username: "{username}", password: "{password}") {{ token }} }}"#
    );
    let resp = schema.execute(q.as_str()).await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    resp.data.into_json().unwrap()["login"]["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn principal_from_token(jwt: &JwtConfig, token: &str) -> Principal {
    let claims = verify_jwt(jwt, token).expect("verify");
    Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("sub uuid")),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(claims.sid.parse().expect("sid uuid"))),
    }
}

async fn create_user(
    schema: &noflex_graphql::NoflexSchema,
    admin: Principal,
    username: &str,
) -> Principal {
    let q = format!(
        r#"mutation {{ createUser(username: "{username}", password: "pw1", role: USER) {{ id }} }}"#
    );
    let resp = schema
        .execute(Request::new(q).data(admin).data(as_request_user(admin)))
        .await;
    assert!(resp.errors.is_empty(), "create user: {:?}", resp.errors);
    let token = login(schema, username, "pw1").await;
    let jwt = JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config");
    principal_from_token(&jwt, &token)
}

struct StubLinkService;

#[async_trait::async_trait]
impl PluginLinkService for StubLinkService {
    async fn start_link(
        &self,
        _user: UserId,
        plugin_id: &str,
    ) -> anyhow::Result<PluginLinkChallenge> {
        Ok(PluginLinkChallenge {
            auth_url: format!("https://provider.example/authorize?client={plugin_id}"),
            state: "stub-state-1".to_string(),
            user_code: Some("ABCD-1234".to_string()),
        })
    }

    async fn unlink(&self, _user: UserId, _plugin_id: &str) -> anyhow::Result<()> {
        anyhow::bail!("upstream unreachable")
    }
}

#[tokio::test]
async fn link_surfaces_reject_anonymous() {
    let (schema, jwt, _store) = authed_schema().await;
    setup_admin(&schema, &jwt).await;

    for query in [
        r#"{ myPluginLinks { pluginId } }"#,
        r#"mutation { startPluginLink(pluginId: "trakt") { authUrl } }"#,
        r#"mutation { unlinkPluginAccount(pluginId: "trakt") { pluginId } }"#,
        r#"{ linkablePlugins { id } }"#,
        r#"mutation { enqueuePluginTask(pluginId: "trakt", task: "sync") }"#,
    ] {
        let resp = schema.execute(query).await;
        assert!(
            resp.errors
                .iter()
                .any(|error| error.message == "authentication required"),
            "anonymous must be rejected: {query}: {:?}",
            resp.errors
        );
    }
}

#[tokio::test]
async fn enqueue_plugin_task_requires_admin() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let bob = create_user(&schema, admin, "bob").await;

    let resp = schema
        .execute(
            Request::new(r#"mutation { enqueuePluginTask(pluginId: "trakt", task: "sync") }"#)
                .data(bob)
                .data(as_request_user(bob)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "administrator required"),
        "non-admin enqueue: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn enqueue_plugin_task_unknown_plugin_errors() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;

    // The registry is empty, so the enqueue never reaches a task queue.
    let resp = schema
        .execute(
            Request::new(r#"mutation { enqueuePluginTask(pluginId: "trakt", task: "sync") }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert_eq!(resp.errors.len(), 1, "enqueue: {:?}", resp.errors);
    assert!(
        resp.errors[0].message.contains("not found"),
        "enqueue: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn start_plugin_link_delegates_to_service() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;

    let resp = schema
        .execute(
            Request::new(
                r#"mutation { startPluginLink(pluginId: "trakt") { authUrl state userCode } }"#,
            )
            .data(admin)
            .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "start: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(
        data["startPluginLink"]["authUrl"],
        "https://provider.example/authorize?client=trakt"
    );
    assert_eq!(data["startPluginLink"]["state"], "stub-state-1");
    assert_eq!(data["startPluginLink"]["userCode"], "ABCD-1234");
}

#[tokio::test]
async fn start_plugin_link_without_service_is_clean_error() {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let schema = build_schema(SchemaContext {
        storage: Some(store),
        jwt: Some(jwt.clone()),
        ..Default::default()
    });
    let admin = setup_admin(&schema, &jwt).await;

    let resp = schema
        .execute(
            Request::new(r#"mutation { startPluginLink(pluginId: "trakt") { authUrl } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "plugin linking not configured"),
        "missing service: {:?}",
        resp.errors
    );
}

/// The stub's unlink fails, as a revoke against an unreachable provider
/// would: the credentials go regardless.
#[tokio::test]
async fn unlink_drops_stored_secrets() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;

    store
        .set_user_plugin_secret(admin.user_id, "trakt", "access-token", "tok")
        .await
        .expect("seed secret");

    let resp = schema
        .execute(
            Request::new(r#"mutation { unlinkPluginAccount(pluginId: "trakt") { pluginId } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "unlink: {:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["unlinkPluginAccount"]["pluginId"],
        "trakt"
    );
    assert!(
        store
            .list_user_plugin_secrets(admin.user_id, "trakt")
            .await
            .expect("list")
            .is_empty()
    );

    // A replay reports null — nothing left to drop.
    let resp = schema
        .execute(
            Request::new(r#"mutation { unlinkPluginAccount(pluginId: "trakt") { pluginId } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.data.into_json().unwrap()["unlinkPluginAccount"].is_null());
}

#[tokio::test]
async fn my_plugin_links_skips_unloaded_plugins() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;

    // Secrets exist for "trakt", but no plugin by that id is loaded,
    // so the listing stays empty — a link only surfaces while its
    // plugin is.
    store
        .set_user_plugin_secret(admin.user_id, "trakt", "access-token", "tok")
        .await
        .expect("seed secret");

    let resp = schema
        .execute(
            Request::new(r#"{ myPluginLinks { pluginId keys } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "links: {:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["myPluginLinks"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}
