// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use noflex_core::{UserId, UserRole};
use noflex_graphql::{ExternalAuthError, ExternalAuthService};
use noflex_plugin_host::{EngineConfig, PluginRegistry, build_engine};
use noflex_server::external_auth::ServerExternalAuth;
use noflex_storage::{Storage, domain::User};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;
use url::Url;

const PLUGIN_ID: &str = "fixture-external-auth";

const NONCE: &str = "browser-nonce-0123456789abcdefghijklmnopqrstuvwxyz";
const OTHER_NONCE: &str = "attacker-nonce-0123456789abcdefghijklmnopqrstuvwx";

async fn auth_fixture(
    auto_provision: bool,
) -> Option<(ServerExternalAuth, Arc<TursoStore>, UserId)> {
    let wasm = crate::fixtures::wasm("external-auth")?;
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let admin = seed_user(&store, "admin", true).await;
    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(PluginRegistry::new(engine).expect("registry"));
    registry.load_from(&wasm).expect("load fixture");
    let storage: Arc<dyn Storage> = store.clone();
    Some((
        ServerExternalAuth::new(
            registry,
            storage,
            Some("https://media.example/".into()),
            auto_provision,
        ),
        store,
        admin,
    ))
}

async fn seed_user(store: &TursoStore, name: &str, is_admin: bool) -> UserId {
    let user = User {
        id: UserId::new(),
        username: name.into(),
        display_name: None,
        password_hash: String::new(),
        is_admin,
        role: if is_admin {
            UserRole::Admin
        } else {
            UserRole::User
        },
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
        has_pin: false,
    };
    store.upsert_user(&user).await.expect("seed user");
    user.id
}

fn state(challenge: &str) -> String {
    Url::parse(challenge)
        .expect("challenge URL")
        .query_pairs()
        .find_map(|(key, value)| (key == "state").then(|| value.into_owned()))
        .expect("state query")
}

/// Drive start + provider callback, returning the fragment code.
async fn code_for(auth: &ServerExternalAuth, link_user: Option<UserId>, nonce: &str) -> String {
    let challenge = match link_user {
        Some(user) => auth.start_link(PLUGIN_ID, user, nonce).await,
        None => auth.start_login(PLUGIN_ID, nonce).await,
    }
    .expect("start");
    let redirect = auth
        .callback(
            PLUGIN_ID,
            &state(&challenge.authorization_url),
            "fixture-code",
        )
        .await
        .expect("callback");
    let prefix = if link_user.is_some() {
        "/#external-link-code="
    } else {
        "/#external-code="
    };
    redirect
        .strip_prefix(prefix)
        .expect("fragment-only exchange redirect")
        .to_owned()
}

macro_rules! fixture {
    ($auto:expr) => {
        match auth_fixture($auto).await {
            Some(fixture) => fixture,
            None => {
                eprintln!("skipping: could not build external-auth fixture");
                return;
            }
        }
    };
}

#[tokio::test]
async fn login_code_is_single_use_and_bound_to_the_starting_browser() {
    let (auth, store, _admin) = fixture!(true);
    let challenge = auth
        .start_login(PLUGIN_ID, NONCE)
        .await
        .expect("start login");
    assert!(
        challenge
            .authorization_url
            .contains("redirect_uri=https://media.example/auth/fixture-external-auth/callback")
    );
    let state = state(&challenge.authorization_url);
    let redirect = auth
        .callback(PLUGIN_ID, &state, "fixture-code")
        .await
        .expect("callback");
    let code = redirect
        .strip_prefix("/#external-code=")
        .expect("fragment-only exchange redirect");
    let user = auth.complete_login(code, NONCE).await.expect("complete");
    assert!(
        matches!(
            auth.complete_login(code, NONCE).await,
            Err(ExternalAuthError::InvalidCode)
        ),
        "exchange replay"
    );
    assert!(
        auth.callback(PLUGIN_ID, &state, "fixture-code")
            .await
            .is_err(),
        "state replay"
    );
    assert!(
        store.get_user(user).await.expect("get user").is_some(),
        "provisioned user exists"
    );

    // Login CSRF: a code delivered to a browser that didn't start the
    // flow (different nonce) is refused and spent.
    let code = code_for(&auth, None, OTHER_NONCE).await;
    assert!(matches!(
        auth.complete_login(&code, NONCE).await,
        Err(ExternalAuthError::InvalidCode)
    ));
    assert!(matches!(
        auth.complete_login(&code, OTHER_NONCE).await,
        Err(ExternalAuthError::InvalidCode)
    ));
}

#[tokio::test]
async fn login_without_auto_provision_requires_a_linked_identity() {
    let (auth, store, admin) = fixture!(false);
    let users = store.list_users().await.expect("users").len();
    let code = code_for(&auth, None, NONCE).await;
    assert!(matches!(
        auth.complete_login(&code, NONCE).await,
        Err(ExternalAuthError::NotLinked)
    ));
    assert_eq!(store.list_users().await.expect("users").len(), users);

    let code = code_for(&auth, Some(admin), NONCE).await;
    auth.complete_link(&code, NONCE, admin)
        .await
        .expect("link identity");
    let code = code_for(&auth, None, NONCE).await;
    assert_eq!(
        auth.complete_login(&code, NONCE).await.expect("login"),
        admin
    );
}

#[tokio::test]
async fn link_is_completed_only_by_the_starting_user_and_browser() {
    let (auth, store, admin) = fixture!(false);
    let attacker = seed_user(&store, "attacker", false).await;

    // The callback alone links nothing.
    let code = code_for(&auth, Some(attacker), NONCE).await;
    assert!(
        store
            .list_external_identities(attacker)
            .await
            .expect("identities")
            .is_empty()
    );
    // Another signed-in user (the victim the attacker lured through
    // the callback) can't complete it, even with the right nonce...
    assert!(matches!(
        auth.complete_link(&code, NONCE, admin).await,
        Err(ExternalAuthError::InvalidCode)
    ));
    // ...and the attempt spent the code.
    assert!(matches!(
        auth.complete_link(&code, NONCE, attacker).await,
        Err(ExternalAuthError::InvalidCode)
    ));

    let code = code_for(&auth, Some(admin), NONCE).await;
    assert!(matches!(
        auth.complete_link(&code, OTHER_NONCE, admin).await,
        Err(ExternalAuthError::InvalidCode)
    ));
    assert!(
        matches!(
            auth.complete_login(&code_for(&auth, Some(admin), NONCE).await, NONCE)
                .await,
            Err(ExternalAuthError::InvalidCode)
        ),
        "a link code is not a login code"
    );

    let code = code_for(&auth, Some(admin), NONCE).await;
    let linked = auth.complete_link(&code, NONCE, admin).await.expect("link");
    assert_eq!(linked.user_id, admin);
    assert!(
        matches!(
            auth.complete_link(&code, NONCE, admin).await,
            Err(ExternalAuthError::InvalidCode)
        ),
        "link replay"
    );
    assert_eq!(
        store
            .list_external_identities(admin)
            .await
            .expect("identities")
            .len(),
        1
    );

    // The identity now belongs to admin; nobody else can claim it.
    let code = code_for(&auth, Some(attacker), NONCE).await;
    assert!(matches!(
        auth.complete_link(&code, NONCE, attacker).await,
        Err(ExternalAuthError::AlreadyLinked)
    ));
}

#[tokio::test]
async fn callback_rejects_plugin_mismatch_and_consumes_state() {
    let (auth, _store, _admin) = fixture!(true);
    let challenge = auth
        .start_login(PLUGIN_ID, NONCE)
        .await
        .expect("start login");
    let state = state(&challenge.authorization_url);
    assert!(
        auth.callback("other-plugin", &state, "fixture-code")
            .await
            .is_err()
    );
    assert!(
        auth.callback(PLUGIN_ID, &state, "fixture-code")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn start_rejects_short_nonces() {
    let (auth, _store, admin) = fixture!(true);
    assert!(matches!(
        auth.start_login(PLUGIN_ID, "short").await,
        Err(ExternalAuthError::InvalidNonce)
    ));
    assert!(matches!(
        auth.start_link(PLUGIN_ID, admin, "").await,
        Err(ExternalAuthError::InvalidNonce)
    ));
}
