// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{SessionId, UserId};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::{RequestUser, Storage};
use noflex_storage_turso::TursoStore;

fn as_request_user(principal: Principal) -> RequestUser {
    RequestUser {
        user_id: principal.user_id,
        is_admin: principal.is_admin,
    }
}

const TEST_JWT_SECRET: &[u8] = b"sessions-secret-32-bytes-for-test";

async fn authed_schema() -> (noflex_graphql::NoflexSchema, Arc<JwtConfig>) {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let jwt = Arc::new(JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"));
    let schema = build_schema(SchemaContext {
        storage: Some(store),
        jwt: Some(jwt.clone()),
        ..Default::default()
    });
    (schema, jwt)
}

async fn setup_admin(schema: &noflex_graphql::NoflexSchema, jwt: &JwtConfig) -> Principal {
    let resp = schema
        .execute(
            r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId } }"#,
        )
        .await;
    assert!(resp.errors.is_empty(), "wizard: {:?}", resp.errors);
    login(schema, jwt, "admin", "hunter2").await
}

async fn login(
    schema: &noflex_graphql::NoflexSchema,
    jwt: &JwtConfig,
    username: &str,
    password: &str,
) -> Principal {
    let query = format!(
        r#"mutation {{ login(username: "{username}", password: "{password}") {{ token }} }}"#
    );
    let resp = schema.execute(query).await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let claims = verify_jwt(jwt, data["login"]["token"].as_str().unwrap()).expect("verify token");
    Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("sub uuid")),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(claims.sid.parse().expect("sid uuid"))),
    }
}

async fn create_user(schema: &noflex_graphql::NoflexSchema, admin: Principal, username: &str) {
    let query = format!(
        r#"mutation {{ createUser(username: "{username}", password: "pw1", role: USER) {{ id }} }}"#
    );
    let resp = schema
        .execute(Request::new(query).data(admin).data(as_request_user(admin)))
        .await;
    assert!(resp.errors.is_empty(), "createUser: {:?}", resp.errors);
}

/// The caller's session ids, via `mySessions`.
async fn my_session_ids(
    schema: &noflex_graphql::NoflexSchema,
    principal: Principal,
) -> Vec<String> {
    let resp = schema
        .execute(
            Request::new("{ mySessions { id userId } }")
                .data(principal)
                .data(as_request_user(principal)),
        )
        .await;
    assert!(resp.errors.is_empty(), "mySessions: {:?}", resp.errors);
    resp.data.into_json().unwrap()["mySessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn my_sessions_lists_only_the_callers_sessions() {
    let (schema, jwt) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    // A second login for the same user: wizard + two logins = three
    // sessions owned by admin.
    login(&schema, &jwt, "admin", "hunter2").await;
    create_user(&schema, admin, "bob").await;
    let bob = login(&schema, &jwt, "bob", "pw1").await;

    let admin_sessions = my_session_ids(&schema, admin).await;
    assert_eq!(admin_sessions.len(), 3);
    let bob_sessions = my_session_ids(&schema, bob).await;
    assert_eq!(bob_sessions.len(), 1);
    assert!(!admin_sessions.contains(&bob_sessions[0]));
}

#[tokio::test]
async fn session_queries_require_authentication_and_admin() {
    let (schema, jwt) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    create_user(&schema, admin, "bob").await;
    let bob = login(&schema, &jwt, "bob", "pw1").await;

    let resp = schema.execute("{ mySessions { id } }").await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "anonymous mySessions: {:?}",
        resp.errors
    );
    let resp = schema.execute("{ sessions { id } }").await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "anonymous sessions: {:?}",
        resp.errors
    );
    let resp = schema
        .execute(
            Request::new("{ sessions { id } }")
                .data(bob)
                .data(as_request_user(bob)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "administrator required"),
        "non-admin sessions: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn admin_sessions_spans_users_and_filters_by_user() {
    let (schema, jwt) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    create_user(&schema, admin, "bob").await;
    let bob = login(&schema, &jwt, "bob", "pw1").await;

    let resp = schema
        .execute(
            Request::new("{ sessions { userId } }")
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "sessions: {:?}", resp.errors);
    let all = resp.data.into_json().unwrap()["sessions"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(all.len(), 3);

    let query = format!(r#"{{ sessions(userId: "{}") {{ userId }} }}"#, bob.user_id);
    let resp = schema
        .execute(Request::new(query).data(admin).data(as_request_user(admin)))
        .await;
    assert!(
        resp.errors.is_empty(),
        "sessions(userId): {:?}",
        resp.errors
    );
    let bobs = resp.data.into_json().unwrap()["sessions"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(bobs.len(), 1);
    assert_eq!(bobs[0]["userId"].as_str().unwrap(), bob.user_id.to_string());
}

#[tokio::test]
async fn revoke_session_self_and_cross_user_protection() {
    let (schema, jwt) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    create_user(&schema, admin, "bob").await;
    let bob = login(&schema, &jwt, "bob", "pw1").await;
    login(&schema, &jwt, "bob", "pw1").await;

    // Bob revokes his second session (the one that isn't his current
    // principal's).
    let bobs = my_session_ids(&schema, bob).await;
    assert_eq!(bobs.len(), 2);
    let current = bob.session_id.expect("login session").to_string();
    let other = bobs.iter().find(|id| **id != current).unwrap().clone();
    let mutation = format!(r#"mutation {{ revokeSession(id: "{other}") {{ id }} }}"#);
    let resp = schema
        .execute(Request::new(mutation).data(bob).data(as_request_user(bob)))
        .await;
    assert!(resp.errors.is_empty(), "revokeSession: {:?}", resp.errors);
    assert_eq!(resp.data.into_json().unwrap()["revokeSession"]["id"], other);
    assert_eq!(my_session_ids(&schema, bob).await, vec![current]);

    // Bob probing an admin session gets the same "session not found"
    // as a missing row, and the victim session survives.
    let admin_sessions = my_session_ids(&schema, admin).await;
    let mutation = format!(
        r#"mutation {{ revokeSession(id: "{}") {{ id }} }}"#,
        admin_sessions[0]
    );
    let resp = schema
        .execute(Request::new(mutation).data(bob).data(as_request_user(bob)))
        .await;
    assert!(
        resp.errors.iter().any(|e| e.message == "session not found"),
        "cross-user revoke: {:?}",
        resp.errors
    );
    let resp = schema
        .execute(
            Request::new("{ sessions { id } }")
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    let remaining = resp.data.into_json().unwrap()["sessions"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(remaining, admin_sessions.len() + 1);
}

#[tokio::test]
async fn revoke_user_sessions_is_admin_only_and_counts() {
    let (schema, jwt) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    create_user(&schema, admin, "bob").await;
    let bob = login(&schema, &jwt, "bob", "pw1").await;
    login(&schema, &jwt, "bob", "pw1").await;
    let mut bobs = my_session_ids(&schema, bob).await;

    let mutation = format!(
        r#"mutation {{ revokeUserSessions(userId: "{}") {{ id }} }}"#,
        admin.user_id
    );
    let resp = schema
        .execute(Request::new(mutation).data(bob).data(as_request_user(bob)))
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "administrator required"),
        "non-admin revokeUserSessions: {:?}",
        resp.errors
    );

    let mutation = format!(
        r#"mutation {{ revokeUserSessions(userId: "{}") {{ id }} }}"#,
        bob.user_id
    );
    let resp = schema
        .execute(
            Request::new(mutation)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors.is_empty(),
        "revokeUserSessions: {:?}",
        resp.errors
    );
    let mut revoked: Vec<String> = resp.data.into_json().unwrap()["revokeUserSessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["id"].as_str().unwrap().to_owned())
        .collect();
    revoked.sort();
    bobs.sort();
    assert_eq!(revoked, bobs);
    let resp = schema
        .execute(
            Request::new("{ sessions { id } }")
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert_eq!(
        resp.data.into_json().unwrap()["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn refresh_token_hash_is_not_selectable() {
    let (schema, jwt) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;

    let resp = schema
        .execute(
            Request::new("{ mySessions { refreshTokenHash } }")
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        !resp.errors.is_empty(),
        "selecting refreshTokenHash should fail validation"
    );
}
