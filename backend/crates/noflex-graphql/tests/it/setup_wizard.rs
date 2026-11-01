// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use noflex_auth::{JwtConfig, verify_jwt};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::store::Storage;
use noflex_storage_turso::TursoStore;

const TEST_JWT_SECRET: &[u8] = b"test-secret-32-bytes-please-use-it";

#[tokio::test]
async fn complete_setup_wizard_issues_decodable_jwt() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store)),
        jwt: Some(jwt.clone()),
        ..Default::default()
    });

    let mutation = r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId accessToken refreshToken } }"#;
    let resp = schema.execute(mutation).await;
    assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
    let data = resp.data.into_json().expect("data json");
    let user_id = data["completeSetupWizard"]["userId"]
        .as_str()
        .expect("userId string")
        .to_string();
    let access_token = data["completeSetupWizard"]["accessToken"]
        .as_str()
        .expect("accessToken string");
    assert!(!access_token.is_empty(), "access_token must be non-empty");
    let refresh_token = data["completeSetupWizard"]["refreshToken"]
        .as_str()
        .expect("refreshToken string");
    assert!(!refresh_token.is_empty(), "refresh_token must be non-empty");

    let claims = verify_jwt(&jwt, access_token).expect("verify jwt");
    assert_eq!(claims.sub, user_id);
    assert!(claims.is_admin, "wizard user is admin");
}

#[tokio::test]
async fn complete_setup_wizard_issued_token_passes_auth_middleware() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store)),
        jwt: Some(jwt.clone()),
        ..Default::default()
    });

    let mutation =
        r#"mutation { completeSetupWizard(username: "ops", password: "sekret") { accessToken } }"#;
    let resp = schema.execute(mutation).await;
    assert!(resp.errors.is_empty(), "errors: {:?}", resp.errors);
    let access_token = resp.data.into_json().unwrap()["completeSetupWizard"]["accessToken"]
        .as_str()
        .unwrap()
        .to_string();

    let claims = verify_jwt(&jwt, &access_token).expect("verify");
    let parsed = uuid::Uuid::parse_str(&claims.sub).expect("sub is uuid");
    assert_ne!(parsed, uuid::Uuid::nil());
}

#[tokio::test]
async fn complete_setup_wizard_rejects_when_users_already_exist() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store)),
        jwt: Some(jwt),
        ..Default::default()
    });

    let first = r#"mutation { completeSetupWizard(username: "root", password: "pw") { userId } }"#;
    let resp = schema.execute(first).await;
    assert!(resp.errors.is_empty(), "first: {:?}", resp.errors);

    let second =
        r#"mutation { completeSetupWizard(username: "root2", password: "pw") { userId } }"#;
    let resp = schema.execute(second).await;
    assert!(
        !resp.errors.is_empty(),
        "second wizard must be rejected: {:?}",
        resp.data.into_json()
    );
    assert!(
        resp.errors[0]
            .message
            .contains("server is already configured"),
        "msg: {}",
        resp.errors[0].message
    );
}

#[tokio::test]
async fn complete_setup_wizard_rejects_empty_credentials() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let schema = build_schema(SchemaContext {
        storage: Some(Arc::new(store)),
        jwt: Some(jwt),
        ..Default::default()
    });

    let mutation = r#"mutation { completeSetupWizard(username: "", password: "pw") { userId } }"#;
    let resp = schema.execute(mutation).await;
    assert!(
        !resp.errors.is_empty(),
        "empty username must be rejected: {:?}",
        resp.data.into_json()
    );
    assert!(
        resp.errors[0]
            .message
            .contains("username and password are required")
    );
}

#[tokio::test]
async fn concurrent_setup_wizard_calls_create_exactly_one_admin() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let schema = build_schema(SchemaContext {
        storage: Some(storage.clone()),
        jwt: Some(jwt),
        ..Default::default()
    });

    // Both calls pass the cheap "no users yet" pre-check before either
    // finishes hashing, so only the atomic insert can arbitrate.
    let first = schema.execute(
        r#"mutation { completeSetupWizard(username: "first", password: "pw-one") { userId } }"#,
    );
    let second = schema.execute(
        r#"mutation { completeSetupWizard(username: "second", password: "pw-two") { userId } }"#,
    );
    let (first, second) = tokio::join!(first, second);
    let succeeded = [&first, &second]
        .iter()
        .filter(|resp| resp.errors.is_empty())
        .count();
    assert_eq!(succeeded, 1, "first: {first:?}\nsecond: {second:?}");
    let loser = if first.errors.is_empty() {
        &second
    } else {
        &first
    };
    assert!(
        loser.errors[0]
            .message
            .contains("server is already configured"),
        "{:?}",
        loser.errors
    );
    let users = storage.list_users().await.expect("list users");
    assert_eq!(users.len(), 1);
    assert!(users[0].is_admin);
}
