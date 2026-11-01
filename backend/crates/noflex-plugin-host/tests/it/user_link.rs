// SPDX-License-Identifier: AGPL-3.0-only
//
// Tests for the `user-link` hook and the `host-secrets.read-for-user`
// import, driven end-to-end through the fixture guest
// (`tests/fixtures/user-link/`). The fixture returns a canned link
// challenge and canned per-user secrets; a `callback_query` of
// `read=<key>` makes it call `read-for-user` from inside the guest and
// report the outcome as a `read-result` secret, so the host-side gates
// (declared key, parseable user id, configured storage, stored value)
// are all exercised through the real wasmtime dispatch.
//
// The fixture component is built on demand (cargo, wasm32-wasip2) when
// the artifact is missing; the tests self-skip if the toolchain can't
// produce it (e.g. a host without the target installed).

use std::sync::Arc;

use noflex_core::UserId;
use noflex_plugin_host::{
    EngineConfig, LoadedPlugin, PluginId, PluginRegistry, build_engine,
    user_link::exports::noflex::plugin::user_link::UserSecret,
};
use noflex_storage::Storage;
use noflex_storage_turso::TursoStore;

const PLUGIN_ID: &str = "fixture-user-link";
const FIXTURE_STATE: &str = "fixture-state-123";

fn registry_with(
    storage: Option<Arc<dyn Storage>>,
    wasm: &std::path::Path,
) -> (PluginRegistry, Arc<LoadedPlugin>) {
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    let registry = match storage {
        Some(s) => registry.with_storage(s),
        None => registry,
    };
    registry
        .load_from(wasm)
        .unwrap_or_else(|e| panic!("load user-link fixture: {e}"));
    let plugin = registry
        .get_plugin(&PluginId(PLUGIN_ID.to_string()))
        .expect("fixture registered");
    (registry, plugin)
}

/// Drive the fixture's `read=<key>` command and return its encoded
/// `read-for-user` outcome (`ok:<value>` / `err:<kind>`).
async fn read_for_user(plugin: &LoadedPlugin, user: &str, key: &str) -> String {
    let secrets = plugin
        .call_link_complete(
            user.to_string(),
            FIXTURE_STATE.to_string(),
            format!("read={key}"),
        )
        .await
        .expect("dispatch succeeds")
        .expect("read probe succeeds");
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0].key, "read-result");
    secrets[0].value.clone()
}

#[tokio::test]
async fn link_hooks_round_trip() {
    let Some(wasm) = crate::fixtures::wasm("user-link") else {
        eprintln!("skipping: could not build the fixture-user-link plugin");
        return;
    };
    let (_registry, plugin) = registry_with(None, &wasm);
    let user = UserId::new().to_string();

    let challenge = plugin
        .call_link_start(user.clone())
        .await
        .expect("dispatch succeeds")
        .expect("link-start succeeds");
    assert_eq!(challenge.state, FIXTURE_STATE);
    assert!(
        challenge.auth_url.contains(&user),
        "auth url targets the linking user: {}",
        challenge.auth_url
    );

    let secrets = plugin
        .call_link_complete(user.clone(), challenge.state, "code=abc".to_string())
        .await
        .expect("dispatch succeeds")
        .expect("link-complete succeeds");
    let as_pairs: Vec<(&str, &str)> = secrets
        .iter()
        .map(|UserSecret { key, value }| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        as_pairs,
        vec![
            ("access-token", "fixture-token-abc"),
            ("refresh-token", "fixture-refresh-def"),
        ]
    );

    let rejected = plugin
        .call_link_complete(user, "bogus-state".to_string(), "code=abc".to_string())
        .await
        .expect("dispatch succeeds");
    assert!(
        matches!(
            rejected,
            Err(noflex_plugin_host::user_link::noflex::plugin::types::PluginError::BadInput(_))
        ),
        "a foreign state is a plugin error: {rejected:?}"
    );
}

#[tokio::test]
async fn read_for_user_paths() {
    let Some(wasm) = crate::fixtures::wasm("user-link") else {
        eprintln!("skipping: could not build the fixture-user-link plugin");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let store = TursoStore::open(dir.path().join("secrets.db"))
        .await
        .expect("open store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let (_registry, plugin) = registry_with(Some(Arc::clone(&storage)), &wasm);

    let user = UserId::new();
    storage
        .upsert_user(&noflex_storage::domain::User {
            id: user,
            name: "linker".to_string(),
            password_hash: String::new(),
            is_admin: false,
            role: noflex_core::UserRole::User,
            created_at: time::OffsetDateTime::now_utc(),
        })
        .await
        .expect("seed user");
    storage
        .set_user_plugin_secret(user, PLUGIN_ID, "access-token", "stored-token-xyz")
        .await
        .expect("seed user secret");
    let foreign_user = UserId::new();
    storage
        .upsert_user(&noflex_storage::domain::User {
            id: foreign_user,
            name: "foreign".to_string(),
            password_hash: String::new(),
            is_admin: false,
            role: noflex_core::UserRole::User,
            created_at: time::OffsetDateTime::now_utc(),
        })
        .await
        .expect("seed foreign user");
    storage
        .set_user_plugin_secret(foreign_user, PLUGIN_ID, "access-token", "foreign-token")
        .await
        .expect("seed foreign secret");

    // Granted: declared key, known user, stored value.
    assert_eq!(
        read_for_user(&plugin, &user.to_string(), "access-token").await,
        "ok:stored-token-xyz"
    );
    // Undeclared key: the manifest's `user_secrets_read` lists only
    // `access-token`.
    assert_eq!(
        read_for_user(&plugin, &user.to_string(), "refresh-token").await,
        "err:capability-denied"
    );
    let cross_user = plugin
        .call_link_complete(
            user.to_string(),
            FIXTURE_STATE.to_string(),
            format!("read-as={foreign_user}:access-token"),
        )
        .await
        .expect("dispatch succeeds")
        .expect("probe succeeds");
    assert_eq!(cross_user[0].value, "err:capability-denied");

    // Declared key, but the user never linked.
    assert_eq!(
        read_for_user(&plugin, &UserId::new().to_string(), "access-token").await,
        "err:unavailable"
    );
    // Garbage user id.
    assert_eq!(
        read_for_user(&plugin, "not-a-uuid", "access-token").await,
        "err:bad-input"
    );
}

#[tokio::test]
async fn read_for_user_without_storage_is_unavailable() {
    let Some(wasm) = crate::fixtures::wasm("user-link") else {
        eprintln!("skipping: could not build the fixture-user-link plugin");
        return;
    };
    let (_registry, plugin) = registry_with(None, &wasm);
    assert_eq!(
        read_for_user(&plugin, &UserId::new().to_string(), "access-token").await,
        "err:unavailable"
    );
}

async fn seeded_store(dir: &std::path::Path, users: &[UserId]) -> Arc<dyn Storage> {
    let store = TursoStore::open(dir.join("tasks.db"))
        .await
        .expect("open store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    for (i, user) in users.iter().enumerate() {
        storage
            .upsert_user(&noflex_storage::domain::User {
                id: *user,
                name: format!("user-{i}"),
                password_hash: String::new(),
                is_admin: false,
                role: noflex_core::UserRole::User,
                created_at: time::OffsetDateTime::now_utc(),
            })
            .await
            .expect("seed user");
    }
    storage
}

/// Wait until the `write-token` task has finished `runs` invocations.
async fn wait_for_task_runs(plugin: &LoadedPlugin, runs: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let done = plugin
            .health
            .read()
            .unwrap()
            .tasks
            .get("write-token")
            .is_some_and(|t| t.total_runs >= runs);
        if done {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "write-token never ran {runs} time(s)"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

async fn enqueue_write(plugin: &LoadedPlugin, caller: UserId, target: UserId) {
    let outcome = plugin
        .call_link_complete(
            caller.to_string(),
            FIXTURE_STATE.to_string(),
            format!("enqueue-write={target}"),
        )
        .await
        .expect("dispatch succeeds")
        .expect("probe succeeds");
    assert_eq!(outcome[0].value, "ok:enqueued");
}

async fn stored_token(storage: &Arc<dyn Storage>, user: UserId) -> Option<String> {
    storage
        .user_plugin_secret(user, PLUGIN_ID, "access-token")
        .await
        .expect("read secret")
        .map(|s| s.value)
}

#[tokio::test]
async fn background_task_runs_bound_to_the_enqueuing_user() {
    let Some(wasm) = crate::fixtures::wasm("user-link") else {
        eprintln!("skipping: could not build the fixture-user-link plugin");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let (alice, bob) = (UserId::new(), UserId::new());
    let storage = seeded_store(dir.path(), &[alice, bob]).await;
    let (registry, plugin) = registry_with(Some(Arc::clone(&storage)), &wasm);

    // Alice's link call enqueues a write for herself: the background run
    // acts for her, so the write lands (the Trakt device-flow shape).
    enqueue_write(&plugin, alice, alice).await;
    wait_for_task_runs(&plugin, 1).await;
    assert_eq!(
        stored_token(&storage, alice).await.as_deref(),
        Some("from-background")
    );

    // Alice's call cannot smuggle a write for Bob through the task.
    enqueue_write(&plugin, alice, bob).await;
    wait_for_task_runs(&plugin, 2).await;
    assert_eq!(stored_token(&storage, bob).await, None);

    // A host-side enqueue binds no one, so its run cannot write either.
    registry
        .enqueue_task(PLUGIN_ID, "write-token", Some(bob.to_string()))
        .await
        .expect("host enqueue");
    wait_for_task_runs(&plugin, 3).await;
    assert_eq!(stored_token(&storage, bob).await, None);
    let failures = plugin.health.read().unwrap().tasks["write-token"].consecutive_failures;
    assert_eq!(failures, 2, "both refused writes surface as task errors");
}

#[tokio::test]
async fn concurrent_user_bound_calls_each_see_only_their_own_principal() {
    let Some(wasm) = crate::fixtures::wasm("user-link") else {
        eprintln!("skipping: could not build the fixture-user-link plugin");
        return;
    };
    let dir = tempfile::tempdir().expect("tempdir");
    let users: Vec<UserId> = (0..8).map(|_| UserId::new()).collect();
    let storage = seeded_store(dir.path(), &users).await;
    for user in &users {
        storage
            .set_user_plugin_secret(*user, PLUGIN_ID, "access-token", &format!("token-{user}"))
            .await
            .expect("seed secret");
    }
    let (_registry, plugin) = registry_with(Some(storage), &wasm);

    let calls = users.iter().map(|user| {
        let plugin = Arc::clone(&plugin);
        let user = user.to_string();
        async move {
            let value = read_for_user(&plugin, &user, "access-token").await;
            (user, value)
        }
    });
    for (user, value) in futures::future::join_all(calls).await {
        assert_eq!(value, format!("ok:token-{user}"));
    }
}
