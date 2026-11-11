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

/// Run the setup wizard, then log in so the returned `Principal`
/// carries the session id a real request would.
async fn setup_admin(schema: &noflex_graphql::NoflexSchema, jwt: &JwtConfig) -> Principal {
    let resp = schema
        .execute(
            r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId } }"#,
        )
        .await;
    assert!(resp.errors.is_empty(), "wizard: {:?}", resp.errors);
    let (token, _) = login(schema, "admin", "hunter2").await;
    principal_from_token(jwt, &token)
}

/// Login and return the `(access, refresh)` pair.
async fn login(
    schema: &noflex_graphql::NoflexSchema,
    username: &str,
    password: &str,
) -> (String, String) {
    let q = format!(
        r#"mutation {{ login(username: "{username}", password: "{password}") {{ token refreshToken }} }}"#
    );
    let resp = schema.execute(q.as_str()).await;
    assert!(
        resp.errors.is_empty(),
        "login {username}: {:?}",
        resp.errors
    );
    let data = resp.data.into_json().unwrap();
    (
        data["login"]["token"].as_str().unwrap().to_string(),
        data["login"]["refreshToken"].as_str().unwrap().to_string(),
    )
}

fn principal_from_token(jwt: &JwtConfig, token: &str) -> Principal {
    let claims = verify_jwt(jwt, token).expect("verify");
    Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("sub uuid")),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(claims.sid.parse().expect("sid uuid"))),
    }
}

/// Admin-create a user; returns the new user's id.
async fn create_user(
    schema: &noflex_graphql::NoflexSchema,
    admin: Principal,
    username: &str,
    password: &str,
    role: &str,
) -> String {
    let q = format!(
        r#"mutation {{ createUser(username: "{username}", password: "{password}", role: {role}) {{ id name role isAdmin }} }}"#
    );
    let resp = schema
        .execute(
            Request::new(q.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "createUser: {:?}", resp.errors);
    resp.data.into_json().unwrap()["createUser"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn admin_surfaces_reject_anonymous() {
    let (schema, jwt, _store) = authed_schema().await;
    setup_admin(&schema, &jwt).await;

    let bogus = UserId::new();
    let calls = [
        r#"{ users { id } }"#.to_string(),
        r#"mutation { createUser(username: "x", password: "y", role: USER) { id } }"#.to_string(),
        format!(r#"mutation {{ updateUser(id: "{bogus}", role: USER) {{ id }} }}"#),
        format!(r#"mutation {{ deleteUser(id: "{bogus}") {{ user {{ id }} }} }}"#),
        format!(r#"{{ userLibraryAccess(userId: "{bogus}") {{ id }} }}"#),
        r#"mutation { changePassword(currentPassword: "a", newPassword: "b") }"#.to_string(),
    ];
    for q in calls {
        let resp = schema.execute(q.as_str()).await;
        assert!(
            resp.errors
                .iter()
                .any(|e| e.message == "authentication required"),
            "anonymous must be rejected: {q}: {:?}",
            resp.errors
        );
    }
}

#[tokio::test]
async fn admin_surfaces_reject_non_admin() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    create_user(&schema, admin, "bob", "pw1", "USER").await;
    let (token, _) = login(&schema, "bob", "pw1").await;
    let bob = principal_from_token(&jwt, &token);

    let calls = [
        r#"{ users { id } }"#.to_string(),
        r#"mutation { createUser(username: "x", password: "y", role: USER) { id } }"#.to_string(),
        format!(
            r#"mutation {{ updateUser(id: "{}", role: ADMIN) {{ id }} }}"#,
            bob.user_id
        ),
        format!(
            r#"mutation {{ deleteUser(id: "{}") {{ user {{ id }} }} }}"#,
            admin.user_id
        ),
        format!(
            r#"{{ userLibraryAccess(userId: "{}") {{ id }} }}"#,
            bob.user_id
        ),
    ];
    for q in calls {
        let resp = schema
            .execute(
                Request::new(q.as_str())
                    .data(bob)
                    .data(as_request_user(bob)),
            )
            .await;
        assert!(
            resp.errors
                .iter()
                .any(|e| e.message == "administrator required"),
            "non-admin must be rejected: {q}: {:?}",
            resp.errors
        );
    }

    // changePassword is self-service, not admin-gated.
    let resp = schema
        .execute(
            Request::new(
                r#"mutation { changePassword(currentPassword: "pw1", newPassword: "pw2") }"#,
            )
            .data(bob)
            .data(as_request_user(bob)),
        )
        .await;
    assert!(resp.errors.is_empty(), "changePassword: {:?}", resp.errors);
}

#[tokio::test]
async fn create_user_then_login() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;

    let id = create_user(&schema, admin, "bob", "pw1", "USER").await;
    let (token, _) = login(&schema, "bob", "pw1").await;
    let claims = verify_jwt(&jwt, &token).expect("verify");
    assert_eq!(claims.sub, id);
    assert!(!claims.is_admin);

    let resp = schema
        .execute(
            Request::new(r#"{ users { id name role isAdmin } }"#)
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "users: {:?}", resp.errors);
    let users = resp.data.into_json().unwrap();
    let users = users["users"].as_array().unwrap();
    assert_eq!(users.len(), 2);
    let bob = users
        .iter()
        .find(|u| u["name"] == "bob")
        .expect("bob listed");
    assert_eq!(bob["role"], "USER");
    assert_eq!(bob["isAdmin"], false);
}

#[tokio::test]
async fn create_user_rejects_duplicate_username() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    create_user(&schema, admin, "bob", "pw1", "USER").await;

    let resp = schema
        .execute(
            Request::new(
                r#"mutation { createUser(username: "bob", password: "pw2", role: USER) { id } }"#,
            )
            .data(admin)
            .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "username already taken"),
        "duplicate username: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn create_user_rejects_empty_fields() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;

    for q in [
        r#"mutation { createUser(username: "", password: "pw", role: USER) { id } }"#,
        r#"mutation { createUser(username: "bob", password: "", role: USER) { id } }"#,
    ] {
        let resp = schema
            .execute(Request::new(q).data(admin).data(as_request_user(admin)))
            .await;
        assert!(
            resp.errors
                .iter()
                .any(|e| e.message == "username and password are required"),
            "empty fields must be rejected: {q}: {:?}",
            resp.errors
        );
    }
}

#[tokio::test]
async fn update_user_refuses_demoting_last_admin() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;

    let q = format!(
        r#"mutation {{ updateUser(id: "{}", role: USER) {{ id }} }}"#,
        admin.user_id
    );
    let resp = schema
        .execute(
            Request::new(q.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "cannot demote the last administrator"),
        "demote last admin: {:?}",
        resp.errors
    );

    // With a second admin in place the same demotion succeeds and
    // keeps isAdmin in lockstep with the role.
    create_user(&schema, admin, "admin2", "pw1", "ADMIN").await;
    let q = format!(
        r#"mutation {{ updateUser(id: "{}", role: USER) {{ role isAdmin }} }}"#,
        admin.user_id
    );
    let resp = schema
        .execute(
            Request::new(q.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "demote: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(data["updateUser"]["role"], "USER");
    assert_eq!(data["updateUser"]["isAdmin"], false);
}

#[tokio::test]
async fn delete_user_refuses_self_and_last_admin() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;

    let q = format!(
        r#"mutation {{ deleteUser(id: "{}") {{ user {{ id }} }} }}"#,
        admin.user_id
    );
    let resp = schema
        .execute(
            Request::new(q.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "cannot delete your own account"),
        "delete self: {:?}",
        resp.errors
    );

    // A (fabricated) second admin principal still can't delete the
    // only admin row: the last-admin guard doesn't trust the caller.
    let impostor = Principal {
        user_id: UserId::new(),
        is_admin: true,
        session_id: None,
    };
    let resp = schema
        .execute(
            Request::new(q.as_str())
                .data(impostor)
                .data(as_request_user(impostor)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "cannot delete the last administrator"),
        "delete last admin: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn delete_user_kills_sessions_and_login() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let bob_id = create_user(&schema, admin, "bob", "pw1", "USER").await;
    let (_, bob_refresh) = login(&schema, "bob", "pw1").await;

    let q = format!(
        r#"mutation {{ deleteUser(id: "{bob_id}") {{ user {{ id }} sessions {{ userId }} }} }}"#
    );
    let resp = schema
        .execute(
            Request::new(q.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "deleteUser: {:?}", resp.errors);
    let deleted = resp.data.into_json().unwrap();
    assert_eq!(deleted["deleteUser"]["user"]["id"], bob_id);
    assert_eq!(
        deleted["deleteUser"]["sessions"],
        serde_json::json!([{ "userId": bob_id }])
    );

    let q = format!(r#"mutation {{ refreshSession(refreshToken: "{bob_refresh}") {{ token }} }}"#);
    let resp = schema.execute(q.as_str()).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "invalid refresh token"),
        "deleted user's session must not refresh: {:?}",
        resp.errors
    );

    let resp = schema
        .execute(r#"mutation { login(username: "bob", password: "pw1") { token } }"#)
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "invalid username or password"),
        "deleted user must not log in: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn update_user_library_access_grants_and_revokes() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    let bob_id = create_user(&schema, admin, "bob", "pw1", "USER").await;

    let resp = schema
        .execute(
            Request::new(
                r#"mutation { createLibrary(name: "Films", kind: MOVIES, paths: ["/srv/films"]) { id } }"#,
            )
            .data(admin)
            .data(RequestUser {
                user_id: admin.user_id,
                is_admin: admin.is_admin,
            }),
        )
        .await;
    assert!(resp.errors.is_empty(), "createLibrary: {:?}", resp.errors);
    let lib_id = resp.data.into_json().unwrap()["createLibrary"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let access = format!(r#"{{ userLibraryAccess(userId: "{bob_id}") {{ id name }} }}"#);
    let resp = schema
        .execute(
            Request::new(access.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "access: {:?}", resp.errors);
    assert!(
        resp.data.into_json().unwrap()["userLibraryAccess"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let grant =
        format!(r#"mutation {{ updateUser(id: "{bob_id}", libraryIds: ["{lib_id}"]) {{ id }} }}"#);
    let resp = schema
        .execute(
            Request::new(grant.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "grant: {:?}", resp.errors);

    let resp = schema
        .execute(
            Request::new(access.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    let libs = resp.data.into_json().unwrap();
    let libs = libs["userLibraryAccess"].as_array().unwrap();
    assert_eq!(libs.len(), 1);
    assert_eq!(libs[0]["id"], lib_id);

    let revoke = format!(r#"mutation {{ updateUser(id: "{bob_id}", libraryIds: []) {{ id }} }}"#);
    let resp = schema
        .execute(
            Request::new(revoke.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(resp.errors.is_empty(), "revoke: {:?}", resp.errors);

    let resp = schema
        .execute(
            Request::new(access.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.data.into_json().unwrap()["userLibraryAccess"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Unknown library ids are a clean error, not an FK violation.
    let bogus = noflex_core::LibraryId::new();
    let bad =
        format!(r#"mutation {{ updateUser(id: "{bob_id}", libraryIds: ["{bogus}"]) {{ id }} }}"#);
    let resp = schema
        .execute(
            Request::new(bad.as_str())
                .data(admin)
                .data(as_request_user(admin)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message.starts_with("library not found:")),
        "unknown library: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn change_password_flow() {
    let (schema, jwt, _store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    create_user(&schema, admin, "bob", "pw1", "USER").await;
    let (token1, refresh1) = login(&schema, "bob", "pw1").await;
    let (_token2, refresh2) = login(&schema, "bob", "pw1").await;
    let bob = principal_from_token(&jwt, &token1);

    // Wrong current password.
    let resp = schema
        .execute(
            Request::new(
                r#"mutation { changePassword(currentPassword: "nope", newPassword: "pw2") }"#,
            )
            .data(bob)
            .data(as_request_user(bob)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "incorrect current password"),
        "wrong current: {:?}",
        resp.errors
    );

    // Empty new password.
    let resp = schema
        .execute(
            Request::new(r#"mutation { changePassword(currentPassword: "pw1", newPassword: "") }"#)
                .data(bob)
                .data(as_request_user(bob)),
        )
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "new password is required"),
        "empty new password: {:?}",
        resp.errors
    );

    // Success.
    let resp = schema
        .execute(
            Request::new(
                r#"mutation { changePassword(currentPassword: "pw1", newPassword: "pw2") }"#,
            )
            .data(bob)
            .data(as_request_user(bob)),
        )
        .await;
    assert!(resp.errors.is_empty(), "changePassword: {:?}", resp.errors);
    assert_eq!(resp.data.into_json().unwrap()["changePassword"], true);

    // Old password is dead, new password works.
    let resp = schema
        .execute(r#"mutation { login(username: "bob", password: "pw1") { token } }"#)
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "invalid username or password"),
        "old password must be rejected: {:?}",
        resp.errors
    );
    login(&schema, "bob", "pw2").await;

    // The other session (second login) can no longer refresh; the
    // caller's own session survives.
    let q = format!(r#"mutation {{ refreshSession(refreshToken: "{refresh2}") {{ token }} }}"#);
    let resp = schema.execute(q.as_str()).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "invalid refresh token"),
        "other session must be revoked: {:?}",
        resp.errors
    );
    let q = format!(r#"mutation {{ refreshSession(refreshToken: "{refresh1}") {{ token }} }}"#);
    let resp = schema.execute(q.as_str()).await;
    assert!(
        resp.errors.is_empty(),
        "caller's session must survive: {:?}",
        resp.errors
    );
}

/// `query` as `principal`, signed in the way a request through the server is.
async fn run_as(
    schema: &noflex_graphql::NoflexSchema,
    principal: Principal,
    query: &str,
) -> async_graphql::Response {
    schema
        .execute(
            Request::new(query)
                .data(principal)
                .data(as_request_user(principal)),
        )
        .await
}

fn code(response: &async_graphql::Response) -> Option<String> {
    response.errors.iter().find_map(|error| {
        error
            .extensions
            .as_ref()?
            .get("code")
            .map(|code| code.to_string().trim_matches('"').to_string())
    })
}

#[tokio::test]
async fn delete_account_takes_the_caller_and_nobody_else() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    create_user(&schema, admin, "alice", "pw-alice", "USER").await;
    create_user(&schema, admin, "bob", "pw-bob", "USER").await;
    let (alice_token, alice_refresh) = login(&schema, "alice", "pw-alice").await;
    let alice = principal_from_token(&jwt, &alice_token);
    let (_, bob_refresh) = login(&schema, "bob", "pw-bob").await;

    let resp = run_as(
        &schema,
        alice,
        r#"mutation { deleteAccount(currentPassword: "nope") { user { id } } }"#,
    )
    .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "incorrect current password"),
        "wrong password: {:?}",
        resp.errors
    );
    assert!(store.get_user(alice.user_id).await.unwrap().is_some());

    let resp = run_as(
        &schema,
        alice,
        r#"mutation { deleteAccount(currentPassword: "pw-alice") { user { id } serverErased } }"#,
    )
    .await;
    assert!(resp.errors.is_empty(), "deleteAccount: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    assert_eq!(
        data["deleteAccount"]["user"]["id"],
        alice.user_id.to_string()
    );
    assert_eq!(data["deleteAccount"]["serverErased"], false);

    assert!(store.get_user(alice.user_id).await.unwrap().is_none());
    let q =
        format!(r#"mutation {{ refreshSession(refreshToken: "{alice_refresh}") {{ token }} }}"#);
    assert!(
        !schema.execute(q.as_str()).await.errors.is_empty(),
        "their session is gone"
    );
    let purges = store.pending_account_purges().await.unwrap();
    assert_eq!(
        purges.iter().map(|purge| purge.user_id).collect::<Vec<_>>(),
        [alice.user_id]
    );

    let q = format!(r#"mutation {{ refreshSession(refreshToken: "{bob_refresh}") {{ token }} }}"#);
    let resp = schema.execute(q.as_str()).await;
    assert!(
        resp.errors.is_empty(),
        "bob stays signed in: {:?}",
        resp.errors
    );
    login(&schema, "bob", "pw-bob").await;
    login(&schema, "admin", "hunter2").await;
}

#[tokio::test]
async fn an_account_without_a_password_confirms_with_its_username() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema, &jwt).await;
    let user = noflex_storage::domain::User {
        id: UserId::new(),
        username: "carol".to_string(),
        display_name: None,
        password_hash: String::new(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: time::OffsetDateTime::now_utc(),
        avatar_updated_at: None,
        has_pin: false,
    };
    store.upsert_user(&user).await.unwrap();
    let carol = Principal {
        user_id: user.id,
        is_admin: false,
        session_id: None,
    };

    let resp = run_as(
        &schema,
        carol,
        "{ accountDeletion { passwordRequired erasesServer } }",
    )
    .await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["accountDeletion"],
        serde_json::json!({ "passwordRequired": false, "erasesServer": false })
    );
    let resp = run_as(
        &schema,
        carol,
        r#"mutation { deleteAccount(username: "dave") { user { id } } }"#,
    )
    .await;
    assert!(!resp.errors.is_empty(), "someone else's username");
    let resp = run_as(
        &schema,
        carol,
        r#"mutation { deleteAccount(username: "carol") { user { id } } }"#,
    )
    .await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    assert!(store.get_user(user.id).await.unwrap().is_none());
}

#[tokio::test]
async fn the_last_administrator_takes_the_server_only_when_asked() {
    let (schema, jwt, store) = authed_schema().await;
    let admin = setup_admin(&schema, &jwt).await;
    create_user(&schema, admin, "bob", "pw-bob", "USER").await;
    let (_, bob_refresh) = login(&schema, "bob", "pw-bob").await;

    let resp = run_as(
        &schema,
        admin,
        "{ accountDeletion { passwordRequired erasesServer } }",
    )
    .await;
    assert_eq!(
        resp.data.into_json().unwrap()["accountDeletion"],
        serde_json::json!({ "passwordRequired": true, "erasesServer": true })
    );
    let resp = run_as(
        &schema,
        admin,
        r#"mutation { deleteAccount(currentPassword: "hunter2") { user { id } } }"#,
    )
    .await;
    assert_eq!(code(&resp).as_deref(), Some("LAST_ADMINISTRATOR"));
    assert!(store.get_user(admin.user_id).await.unwrap().is_some());
    assert!(store.pending_server_reset().await.unwrap().is_none());

    let resp = run_as(
        &schema,
        admin,
        r#"mutation { deleteAccount(currentPassword: "hunter2", eraseServer: true) { serverErased } }"#,
    )
    .await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["deleteAccount"]["serverErased"],
        true
    );
    assert!(store.pending_server_reset().await.unwrap().is_some());

    let resp = schema.execute("{ serverInfo { resetting } }").await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    assert_eq!(
        resp.data.into_json().unwrap()["serverInfo"]["resetting"],
        true
    );
    let q = format!(r#"mutation {{ refreshSession(refreshToken: "{bob_refresh}") {{ token }} }}"#);
    let resp = schema.execute(q.as_str()).await;
    assert_eq!(code(&resp).as_deref(), Some("SERVER_RESETTING"));
    let resp = schema
        .execute(r#"mutation { login(username: "bob", password: "pw-bob") { token } }"#)
        .await;
    assert_eq!(code(&resp).as_deref(), Some("SERVER_RESETTING"));
}
