// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{SessionId, UserId};
use noflex_graphql::{SchemaContext, build_schema};
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
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        jwt: Some(jwt.clone()),
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

#[tokio::test]
async fn preference_surfaces_reject_anonymous() {
    let (schema, jwt, _store) = authed_schema().await;
    setup_admin(&schema, &jwt).await;

    for query in [
        r#"{ preferences { key } }"#,
        r#"mutation { setPreference(key: "theme", value: "dark") { key } }"#,
        r#"mutation { deletePreference(key: "theme") }"#,
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
async fn set_then_list_round_trip() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let set = schema
        .execute(
            Request::new(
                r#"mutation { setPreference(key: "playback", value: { quality: "auto", subtitles: { enabled: true, languages: ["en", "ja"] } }) { key value } }"#,
            )
            .data(admin).data(as_request_user(admin)),
        )
        .await;
    assert!(set.errors.is_empty(), "setPreference: {:?}", set.errors);

    let listed = schema
        .execute(
            Request::new(r#"{ preferences { key value } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(listed.errors.is_empty(), "preferences: {:?}", listed.errors);
    let data = listed.data.into_json().unwrap();
    assert_eq!(data["preferences"].as_array().unwrap().len(), 1);
    assert_eq!(data["preferences"][0]["key"], "playback");
    assert_eq!(data["preferences"][0]["value"]["quality"], "auto");
    assert_eq!(
        data["preferences"][0]["value"]["subtitles"]["languages"],
        serde_json::json!(["en", "ja"])
    );
}

#[tokio::test]
async fn set_preference_overwrites() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    for value in ["light", "dark"] {
        let query =
            format!(r#"mutation {{ setPreference(key: "theme", value: "{value}") {{ key }} }}"#);
        let resp = schema
            .execute(Request::new(query).data(admin).data(as_request_user(admin)))
            .await;
        assert!(resp.errors.is_empty(), "setPreference: {:?}", resp.errors);
    }

    let resp = schema
        .execute(
            Request::new(r#"{ preferences { key value } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "preferences: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["preferences"].as_array().unwrap().len(), 1);
    assert_eq!(data["preferences"][0]["value"], "dark");
}

#[tokio::test]
async fn delete_preference_returns_true_then_false() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let set = schema
        .execute(
            Request::new(r#"mutation { setPreference(key: "theme", value: "dark") { key } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(set.errors.is_empty(), "setPreference: {:?}", set.errors);

    for expected in [true, false] {
        let resp = schema
            .execute(
                Request::new(r#"mutation { deletePreference(key: "theme") }"#)
                    .data(admin)
                    .data(as_request_user(admin)),
            )
            .await;
        assert!(
            resp.errors.is_empty(),
            "deletePreference: {:?}",
            resp.errors
        );
        assert_eq!(resp.data.into_json().unwrap()["deletePreference"], expected);
    }
}

#[tokio::test]
async fn preferences_are_isolated_per_user() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let bob = create_user(&schema, admin, "bob").await;

    for (principal, value) in [(admin, "admin"), (bob, "bob")] {
        let query =
            format!(r#"mutation {{ setPreference(key: "profile", value: "{value}") {{ key }} }}"#);
        let resp = schema
            .execute(
                Request::new(query)
                    .data(principal)
                    .data(as_request_user(principal)),
            )
            .await;
        assert!(resp.errors.is_empty(), "setPreference: {:?}", resp.errors);
    }

    for (principal, expected) in [(admin, "admin"), (bob, "bob")] {
        let resp = schema
            .execute(
                Request::new(r#"{ preferences { value } }"#)
                    .data(principal)
                    .data(as_request_user(principal)),
            )
            .await;
        assert!(resp.errors.is_empty(), "preferences: {:?}", resp.errors);
        assert_eq!(
            resp.data.into_json().unwrap()["preferences"][0]["value"],
            expected
        );
    }
}
