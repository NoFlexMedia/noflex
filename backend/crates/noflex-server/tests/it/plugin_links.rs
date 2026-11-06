// SPDX-License-Identifier: AGPL-3.0-only
//
// E11 server-side: the per-user plugin link flow end-to-end —
// `start_link` hands out a pending state, the unauthenticated callback
// route consumes it, and the plugin's secrets land in storage. The
// state machine is driven against a fake `PluginLinkHooks`; one test
// repeats the flow against the real `fixture-user-link` WASM guest
// (self-skipping when the artifact can't be built), and one exercises
// the GQL `myPluginLinks` listing over the same stack.

use std::{collections::HashMap, sync::Arc};

use axum::{
    Router,
    extract::{Path, Query, RawQuery, State},
    routing::get,
};
use noflex_core::UserId;
use noflex_graphql::{PluginLinkChallenge, PluginLinkService, SchemaContext, build_schema};
use noflex_plugin_host::{EngineConfig, PluginRegistry, build_engine};
use noflex_server::plugin_links::{PluginLinkHooks, ServerPluginLinks, callback_response};
use noflex_storage::{domain::User, store::Storage};
use noflex_storage_turso::TursoStore;

const PLUGIN_ID: &str = "fake-link";
const FAKE_STATE: &str = "fake-state-1";

async fn temp_storage() -> Arc<TursoStore> {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    store
}

async fn seed_user(storage: &Arc<TursoStore>) -> UserId {
    let user = UserId::new();
    storage
        .upsert_user(&User {
            id: user,
            username: "linker".to_string(),
            display_name: None,
            password_hash: String::new(),
            is_admin: false,
            role: noflex_core::UserRole::User,
            created_at: time::OffsetDateTime::now_utc(),
            avatar_updated_at: None,
        })
        .await
        .expect("seed user");
    user
}

/// The same handler shape `main.rs` mounts, minus the AppState.
fn callback_router(links: Arc<ServerPluginLinks>) -> Router {
    async fn handler(
        State(links): State<Arc<ServerPluginLinks>>,
        Path(plugin_id): Path<String>,
        RawQuery(raw): RawQuery,
        Query(params): Query<HashMap<String, String>>,
    ) -> axum::response::Response {
        callback_response(
            &links,
            &plugin_id,
            raw.as_deref(),
            params.get("state").map(String::as_str),
        )
        .await
    }
    Router::new()
        .route("/plugins/{plugin_id}/link/callback", get(handler))
        .with_state(links)
}

/// Serve `router` on an ephemeral port and issue one raw GET (no HTTP
/// client in the dependency tree), returning the status and full
/// response text.
async fn raw_get(router: Router, uri: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!("GET {uri} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("write");
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.expect("read");
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .expect("status token")
        .parse()
        .expect("status number");
    (status, text)
}

struct FakeLinkHooks;
struct UndeclaredLinkHooks;

#[async_trait::async_trait]
impl PluginLinkHooks for FakeLinkHooks {
    async fn link_start(
        &self,
        _plugin_id: &str,
        user_id: String,
    ) -> anyhow::Result<PluginLinkChallenge> {
        Ok(PluginLinkChallenge {
            auth_url: format!("https://provider.example/auth?user={user_id}"),
            state: FAKE_STATE.to_string(),
        })
    }

    async fn link_complete(
        &self,
        _plugin_id: &str,
        _user_id: String,
        _state: String,
        callback_query: String,
    ) -> anyhow::Result<Vec<(String, String)>> {
        assert_eq!(callback_query, format!("state={FAKE_STATE}&code=abc"));
        Ok(vec![
            ("access-token".to_string(), "tok-abc".to_string()),
            ("refresh-token".to_string(), "ref-def".to_string()),
        ])
    }

    fn writable_secret_keys(&self, _plugin_id: &str) -> anyhow::Result<Vec<String>> {
        Ok(vec!["access-token".into(), "refresh-token".into()])
    }
}

#[async_trait::async_trait]
impl PluginLinkHooks for UndeclaredLinkHooks {
    async fn link_start(
        &self,
        _plugin_id: &str,
        _user_id: String,
    ) -> anyhow::Result<PluginLinkChallenge> {
        Ok(PluginLinkChallenge {
            auth_url: "https://provider.example/auth".into(),
            state: FAKE_STATE.into(),
        })
    }

    async fn link_complete(
        &self,
        _plugin_id: &str,
        _user_id: String,
        _state: String,
        _callback_query: String,
    ) -> anyhow::Result<Vec<(String, String)>> {
        Ok(vec![("undeclared".into(), "value".into())])
    }

    fn writable_secret_keys(&self, _plugin_id: &str) -> anyhow::Result<Vec<String>> {
        Ok(vec!["access-token".into()])
    }
}

#[tokio::test]
async fn link_flow_rejects_undeclared_secret_keys() {
    let store = temp_storage().await;
    let user = seed_user(&store).await;
    let links = ServerPluginLinks::new(Arc::new(UndeclaredLinkHooks), store.clone());
    links.start_link(user, PLUGIN_ID).await.expect("start link");
    assert!(
        links
            .complete_link(PLUGIN_ID, FAKE_STATE, "code=abc")
            .await
            .is_err()
    );
    assert!(
        store
            .list_user_plugin_secrets(user, PLUGIN_ID)
            .await
            .expect("list secrets")
            .is_empty()
    );
}

#[tokio::test]
async fn link_flow_persists_secrets_and_consumes_state() {
    let store = temp_storage().await;
    let user = seed_user(&store).await;
    let links = Arc::new(ServerPluginLinks::new(
        Arc::new(FakeLinkHooks),
        store.clone(),
    ));

    let challenge = links.start_link(user, PLUGIN_ID).await.expect("start link");
    assert_eq!(challenge.state, FAKE_STATE);

    let (status, body) = raw_get(
        callback_router(links.clone()),
        &format!("/plugins/{PLUGIN_ID}/link/callback?state={FAKE_STATE}&code=abc"),
    )
    .await;
    assert_eq!(status, 200, "callback: {body}");
    assert!(body.contains("Account linked"), "callback body: {body}");

    let secrets = store
        .list_user_plugin_secrets(user, PLUGIN_ID)
        .await
        .expect("list secrets");
    let as_pairs: Vec<(&str, &str)> = secrets
        .iter()
        .map(|s| (s.key.as_str(), s.value.as_str()))
        .collect();
    assert_eq!(
        as_pairs,
        vec![("access-token", "tok-abc"), ("refresh-token", "ref-def")]
    );

    // The consumed state can't replay.
    let (status, _) = raw_get(
        callback_router(links.clone()),
        &format!("/plugins/{PLUGIN_ID}/link/callback?state={FAKE_STATE}&code=abc"),
    )
    .await;
    assert_eq!(status, 400, "replay must fail");

    // A state that was never issued fails the same way.
    let (status, _) = raw_get(
        callback_router(links.clone()),
        &format!("/plugins/{PLUGIN_ID}/link/callback?state=bogus&code=abc"),
    )
    .await;
    assert_eq!(status, 400, "unknown state must fail");
}

#[tokio::test]
async fn callback_rejects_foreign_plugin_and_missing_state() {
    let store = temp_storage().await;
    let user = seed_user(&store).await;
    let links = Arc::new(ServerPluginLinks::new(Arc::new(FakeLinkHooks), store));
    links.start_link(user, PLUGIN_ID).await.expect("start");

    // The pending state is bound to the plugin that issued it; a
    // callback under a different plugin id consumes it and fails.
    let (status, _) = raw_get(
        callback_router(links.clone()),
        &format!("/plugins/other/link/callback?state={FAKE_STATE}&code=abc"),
    )
    .await;
    assert_eq!(status, 400, "plugin mismatch must fail");

    // No `state` parameter at all.
    let (status, _) = raw_get(
        callback_router(links.clone()),
        &format!("/plugins/{PLUGIN_ID}/link/callback?code=abc"),
    )
    .await;
    assert_eq!(status, 400, "missing state must fail");
}

// ---- Real-plugin flow, through the fixture-user-link WASM guest ------

const FIXTURE_PLUGIN_ID: &str = "fixture-user-link";

#[tokio::test]
async fn link_flow_against_real_plugin() {
    let Some(wasm) = crate::fixtures::wasm("user-link") else {
        eprintln!("skipping: could not build the fixture-user-link plugin");
        return;
    };
    let store = temp_storage().await;
    let user = seed_user(&store).await;
    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(
        PluginRegistry::new(engine)
            .expect("registry")
            .with_storage(store.clone()),
    );
    registry
        .load_from(&wasm)
        .unwrap_or_else(|e| panic!("load fixture: {e}"));
    let links = Arc::new(ServerPluginLinks::new(registry.clone(), store.clone()));

    // A loaded plugin without the user-link hook is a clean error, and
    // an unknown plugin id is one too.
    let err = links
        .start_link(user, "not-loaded")
        .await
        .expect_err("unknown plugin must fail");
    assert!(err.to_string().contains("not loaded"), "{err}");

    let challenge = links
        .start_link(user, FIXTURE_PLUGIN_ID)
        .await
        .expect("start link");
    let (status, body) = raw_get(
        callback_router(links.clone()),
        &format!(
            "/plugins/{FIXTURE_PLUGIN_ID}/link/callback?state={}&code=abc",
            challenge.state
        ),
    )
    .await;
    assert_eq!(status, 200, "callback: {body}");

    let secrets = store
        .list_user_plugin_secrets(user, FIXTURE_PLUGIN_ID)
        .await
        .expect("list secrets");
    let as_pairs: Vec<(&str, &str)> = secrets
        .iter()
        .map(|s| (s.key.as_str(), s.value.as_str()))
        .collect();
    assert_eq!(
        as_pairs,
        vec![
            ("access-token", "fixture-token-abc"),
            ("refresh-token", "fixture-refresh-def"),
        ]
    );

    // The GQL listing surfaces key names — never values.
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        plugins: Some(registry),
        ..Default::default()
    });
    let principal = noflex_auth::Principal {
        user_id: user,
        is_admin: false,
        session_id: None,
    };
    let resp = schema
        .execute(
            async_graphql::Request::new(r#"{ myPluginLinks { pluginId keys updatedAt } }"#)
                .data(principal),
        )
        .await;
    assert!(resp.errors.is_empty(), "myPluginLinks: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let links_json = data["myPluginLinks"].as_array().unwrap();
    assert_eq!(links_json.len(), 1);
    assert_eq!(links_json[0]["pluginId"], FIXTURE_PLUGIN_ID);
    assert_eq!(
        links_json[0]["keys"],
        serde_json::json!(["access-token", "refresh-token"])
    );
    assert!(links_json[0]["updatedAt"].is_string());
}
