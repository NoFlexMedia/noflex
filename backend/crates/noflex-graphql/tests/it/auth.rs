// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::{Request, Variables};
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{SessionId, UserId};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::store::Storage;
use noflex_storage_turso::TursoStore;

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

/// Run the setup wizard to create the admin account that `login` targets.
async fn setup_admin(schema: &noflex_graphql::NoflexSchema) {
    let resp = schema
        .execute(
            r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId } }"#,
        )
        .await;
    assert!(resp.errors.is_empty(), "wizard: {:?}", resp.errors);
}

#[tokio::test]
async fn login_success() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema).await;

    let login = r#"mutation { login(username: "admin", password: "hunter2") { token refreshToken user { id name isAdmin } } }"#;
    let resp = schema.execute(login).await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let token = data["login"]["token"].as_str().expect("token").to_string();
    let refresh_token = data["login"]["refreshToken"]
        .as_str()
        .expect("refreshToken")
        .to_string();
    let user = &data["login"]["user"];
    assert_eq!(user["name"], "admin");
    assert_eq!(user["isAdmin"], true);
    assert!(!token.is_empty());
    assert!(!refresh_token.is_empty());

    let claims = verify_jwt(&jwt, &token).expect("verify");
    assert_eq!(claims.sub, user["id"].as_str().unwrap());
    assert!(claims.is_admin);

    // The sid claim resolves to a real, revocable session row holding
    // the hash of the issued refresh token (never the plaintext).
    let sid = SessionId::from_uuid(claims.sid.parse().expect("sid uuid"));
    let session = store
        .get_session(sid)
        .await
        .expect("get_session")
        .expect("session present");
    assert_eq!(
        session.user_id,
        UserId::from_uuid(claims.sub.parse().unwrap())
    );
    assert_eq!(
        session.refresh_token_hash.as_deref(),
        Some(noflex_auth::hash_refresh_token(&refresh_token)).as_deref()
    );
    assert!(session.expires_at > time::OffsetDateTime::now_utc());
}

#[tokio::test]
async fn login_rejects_wrong_password() {
    let (schema, _jwt, _store) = authed_schema().await;
    setup_admin(&schema).await;

    let resp = schema
        .execute(r#"mutation { login(username: "admin", password: "wrong") { token } }"#)
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "invalid username or password"),
        "wrong password: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn login_rejects_oversized_credentials_without_disclosing_details() {
    let (schema, _jwt, _store) = authed_schema().await;
    setup_admin(&schema).await;

    let request = Request::new(
        "mutation Login($password: String!) { login(username: \"admin\", password: $password) { token } }",
    )
    .variables(Variables::from_json(serde_json::json!({
        "password": "x".repeat(noflex_auth::MAX_PASSWORD_BYTES + 1),
    })));
    let resp = schema.execute(request).await;
    assert_eq!(resp.errors.len(), 1);
    assert_eq!(resp.errors[0].message, "invalid username or password");
}

#[tokio::test]
async fn login_rejects_unknown_user() {
    let (schema, _jwt, _store) = authed_schema().await;
    setup_admin(&schema).await;

    let resp = schema
        .execute(r#"mutation { login(username: "ghost", password: "whatever") { token } }"#)
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "invalid username or password"),
        "unknown user: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn logout_revokes_session() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema).await;

    let resp = schema
        .execute(r#"mutation { login(username: "admin", password: "hunter2") { token } }"#)
        .await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    let token = resp.data.into_json().unwrap()["login"]["token"]
        .as_str()
        .unwrap()
        .to_string();
    let claims = verify_jwt(&jwt, &token).expect("verify");
    let sid = SessionId::from_uuid(claims.sid.parse().expect("sid uuid"));
    let user_id = UserId::from_uuid(claims.sub.parse().expect("sub uuid"));

    let principal = Principal {
        user_id,
        is_admin: true,
        session_id: Some(sid),
    };
    let resp = schema
        .execute(Request::new("mutation { logout }").data(principal))
        .await;
    assert!(resp.errors.is_empty(), "logout: {:?}", resp.errors);
    assert_eq!(resp.data.into_json().unwrap()["logout"], true);

    let after = store.get_session(sid).await.expect("get_session");
    assert!(after.is_none(), "session must be revoked");
}

#[tokio::test]
async fn logout_without_a_session_is_a_no_op() {
    let (schema, _jwt, _store) = authed_schema().await;
    setup_admin(&schema).await;

    // An API-key principal: authenticated, but no session backs it.
    let principal = Principal {
        user_id: UserId::new(),
        is_admin: false,
        session_id: None,
    };
    let resp = schema
        .execute(Request::new("mutation { logout }").data(principal))
        .await;
    assert!(
        resp.errors.is_empty(),
        "sessionless logout: {:?}",
        resp.errors
    );
    assert_eq!(resp.data.into_json().unwrap()["logout"], false);
}

#[tokio::test]
async fn logout_requires_auth() {
    let (schema, _jwt, _store) = authed_schema().await;

    // No Principal in the request data → guard refuses.
    let resp = schema.execute("mutation { logout }").await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "authentication required"),
        "unauthenticated logout: {:?}",
        resp.errors
    );
}

/// Login, then pull the token pair out of the response.
async fn login_tokens(schema: &noflex_graphql::NoflexSchema) -> (String, String) {
    let resp = schema
        .execute(
            r#"mutation { login(username: "admin", password: "hunter2") { token refreshToken } }"#,
        )
        .await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    (
        data["login"]["token"].as_str().unwrap().to_string(),
        data["login"]["refreshToken"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn refresh_session_rotates_and_preserves_sid() {
    let (schema, jwt, _store) = authed_schema().await;
    setup_admin(&schema).await;
    let (token, refresh_token) = login_tokens(&schema).await;
    let old_claims = verify_jwt(&jwt, &token).expect("verify login token");

    let refresh = format!(
        r#"mutation {{ refreshSession(refreshToken: "{refresh_token}") {{ token refreshToken user {{ id }} }} }}"#
    );
    let resp = schema.execute(refresh.as_str()).await;
    assert!(resp.errors.is_empty(), "refresh: {:?}", resp.errors);
    let data = resp.data.into_json().unwrap();
    let new_token = data["refreshSession"]["token"].as_str().unwrap();
    let new_refresh = data["refreshSession"]["refreshToken"].as_str().unwrap();
    assert_ne!(new_refresh, refresh_token, "rotation issues a new token");

    let new_claims = verify_jwt(&jwt, new_token).expect("verify refreshed token");
    assert_eq!(new_claims.sub, old_claims.sub);
    assert_eq!(
        new_claims.sid, old_claims.sid,
        "refresh rotates in place: same session id"
    );

    // The old refresh token is dead the moment rotation succeeds; an
    // immediate replay is inside the race grace window, so it is refused
    // without revoking the session.
    let resp = schema.execute(refresh.as_str()).await;
    assert_eq!(
        refresh_code(&resp),
        Some("REFRESH_TOKEN_SUPERSEDED".into()),
        "replayed refresh token: {:?}",
        resp.errors
    );
    let resp = schema.execute(refresh_mutation(new_refresh).as_str()).await;
    assert!(
        resp.errors.is_empty(),
        "an in-grace replay must not revoke the session: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn refresh_session_rejects_unknown_token() {
    let (schema, _jwt, _store) = authed_schema().await;
    setup_admin(&schema).await;

    let resp = schema
        .execute(r#"mutation { refreshSession(refreshToken: "deadbeef") { token } }"#)
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "invalid refresh token"),
        "unknown refresh token: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn refresh_session_rejects_expired_token() {
    let (schema, _jwt, store) = authed_schema().await;
    setup_admin(&schema).await;

    let admin = store
        .get_user_by_username("admin")
        .await
        .expect("get_user_by_username")
        .expect("admin present");
    let now = time::OffsetDateTime::now_utc();
    let session = noflex_storage::domain::Session {
        id: SessionId::new(),
        user_id: admin.id,
        client: None,
        device: None,
        device_id: None,
        app_version: None,
        remote_addr: None,
        created_at: now - time::Duration::days(31),
        last_seen_at: now - time::Duration::days(31),
        expires_at: now - time::Duration::days(1),
        refresh_token_hash: Some(noflex_auth::hash_refresh_token("stale-token")),
        locked_at: None,
    };
    store
        .create_session(&session)
        .await
        .expect("create session");

    let resp = schema
        .execute(r#"mutation { refreshSession(refreshToken: "stale-token") { token } }"#)
        .await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "invalid refresh token"),
        "expired refresh token: {:?}",
        resp.errors
    );

    // The expired session is revoked on the way out.
    let after = store.get_session(session.id).await.expect("get_session");
    assert!(after.is_none(), "expired session must be revoked");
}

#[tokio::test]
async fn refresh_session_fails_after_logout() {
    let (schema, jwt, _store) = authed_schema().await;
    setup_admin(&schema).await;
    let (token, refresh_token) = login_tokens(&schema).await;
    let claims = verify_jwt(&jwt, &token).expect("verify");

    let principal = Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("sub uuid")),
        is_admin: true,
        session_id: Some(SessionId::from_uuid(claims.sid.parse().expect("sid uuid"))),
    };
    let resp = schema
        .execute(Request::new("mutation { logout }").data(principal))
        .await;
    assert!(resp.errors.is_empty(), "logout: {:?}", resp.errors);

    let refresh =
        format!(r#"mutation {{ refreshSession(refreshToken: "{refresh_token}") {{ token }} }}"#);
    let resp = schema.execute(refresh.as_str()).await;
    assert!(
        resp.errors
            .iter()
            .any(|e| e.message == "invalid refresh token"),
        "refresh after logout: {:?}",
        resp.errors
    );
}

fn refresh_mutation(refresh_token: &str) -> String {
    format!(
        r#"mutation {{ refreshSession(refreshToken: "{refresh_token}") {{ token refreshToken }} }}"#
    )
}

/// The `extensions.code` of the single refresh rejection, if any.
fn refresh_code(resp: &async_graphql::Response) -> Option<String> {
    let error = resp.errors.first()?;
    assert_eq!(error.message, "invalid refresh token");
    let ext = serde_json::to_value(error.extensions.as_ref()?).expect("extensions json");
    ext["code"].as_str().map(str::to_string)
}

fn session_of(jwt: &JwtConfig, token: &str) -> SessionId {
    let claims = verify_jwt(jwt, token).expect("verify");
    SessionId::from_uuid(claims.sid.parse().expect("sid uuid"))
}

/// Replay of a rotated-away refresh token outside the grace window: the
/// whole session dies (the legitimate client's current refresh token and
/// access token with it) and the event lands in the activity log.
#[tokio::test]
async fn replayed_refresh_token_revokes_the_session() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema).await;
    let (token, stolen) = login_tokens(&schema).await;
    let sid = session_of(&jwt, &token);

    // The legitimate client rotated a minute ago and holds `current`.
    let current = noflex_auth::generate_refresh_token();
    let rotated_at = time::OffsetDateTime::now_utc() - time::Duration::minutes(1);
    assert!(
        store
            .rotate_refresh_token(
                sid,
                &noflex_auth::hash_refresh_token(&stolen),
                &noflex_auth::hash_refresh_token(&current),
                rotated_at + time::Duration::days(30),
                rotated_at,
            )
            .await
            .expect("rotate")
    );

    let resp = schema
        .execute(
            Request::new(refresh_mutation(&stolen))
                .data(noflex_auth::ClientAddr("203.0.113.9".parse().unwrap())),
        )
        .await;
    assert_eq!(
        refresh_code(&resp),
        Some("REFRESH_TOKEN_REUSED".into()),
        "{:?}",
        resp.errors
    );
    assert!(
        store.get_session(sid).await.expect("get").is_none(),
        "reuse revokes the session, killing its access tokens"
    );
    let resp = schema.execute(refresh_mutation(&current).as_str()).await;
    assert_eq!(
        refresh_code(&resp),
        Some("REFRESH_TOKEN_INVALID".into()),
        "the legitimate client's token dies with the family: {:?}",
        resp.errors
    );

    let activity = store
        .list_activity(
            &[noflex_storage::domain::ActivityKind::RefreshTokenReuse],
            None,
            10,
            None,
        )
        .await
        .expect("activity");
    assert_eq!(activity.items.len(), 1, "reuse is audit-logged once");
    let entry = &activity.items[0];
    assert_eq!(entry.subject.as_deref(), Some(sid.to_string().as_str()));
    let admin = store
        .get_user_by_username("admin")
        .await
        .expect("get admin")
        .expect("admin");
    assert_eq!(entry.actor.as_deref(), Some(admin.id.to_string().as_str()));
    let detail: serde_json::Value =
        serde_json::from_str(entry.detail.as_deref().expect("detail")).expect("detail json");
    assert_eq!(detail["presentedFrom"], "203.0.113.9");
}

/// Inside the grace window only the immediately-previous token is
/// treated as a race; an older one is reuse even seconds after rotation.
#[tokio::test]
async fn grace_window_covers_only_the_previous_token() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema).await;
    let (token, first) = login_tokens(&schema).await;
    let sid = session_of(&jwt, &token);

    let resp = schema.execute(refresh_mutation(&first).as_str()).await;
    assert!(resp.errors.is_empty(), "first refresh: {:?}", resp.errors);
    let second = resp.data.into_json().unwrap()["refreshSession"]["refreshToken"]
        .as_str()
        .unwrap()
        .to_string();
    let resp = schema.execute(refresh_mutation(&second).as_str()).await;
    assert!(resp.errors.is_empty(), "second refresh: {:?}", resp.errors);

    let resp = schema.execute(refresh_mutation(&first).as_str()).await;
    assert_eq!(
        refresh_code(&resp),
        Some("REFRESH_TOKEN_REUSED".into()),
        "{:?}",
        resp.errors
    );
    assert!(store.get_session(sid).await.expect("get").is_none());
}

/// Two refreshes racing with one token: exactly one wins, the loser is
/// told it was superseded, and the session survives under the winner.
#[tokio::test]
async fn concurrent_refreshes_with_one_token_do_not_revoke() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema).await;
    let (token, refresh_token) = login_tokens(&schema).await;
    let sid = session_of(&jwt, &token);

    let q = refresh_mutation(&refresh_token);
    let (a, b) = tokio::join!(schema.execute(q.as_str()), schema.execute(q.as_str()));
    let (winner, loser) = match (a.errors.is_empty(), b.errors.is_empty()) {
        (true, false) => (a, b),
        (false, true) => (b, a),
        outcome => panic!("exactly one refresh must win, got {outcome:?}"),
    };
    assert_eq!(
        refresh_code(&loser),
        Some("REFRESH_TOKEN_SUPERSEDED".into()),
        "{:?}",
        loser.errors
    );
    assert!(store.get_session(sid).await.expect("get").is_some());
    let next = winner.data.into_json().unwrap()["refreshSession"]["refreshToken"]
        .as_str()
        .unwrap()
        .to_string();
    let resp = schema.execute(refresh_mutation(&next).as_str()).await;
    assert!(
        resp.errors.is_empty(),
        "winner's token works: {:?}",
        resp.errors
    );
}

/// Past the absolute lifetime a session cannot refresh even though its
/// sliding expiry is still in the future.
#[tokio::test]
async fn refresh_fails_after_the_absolute_session_lifetime() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema).await;
    let admin = store
        .get_user_by_username("admin")
        .await
        .expect("get admin")
        .expect("admin");
    let now = time::OffsetDateTime::now_utc();
    let session = noflex_storage::domain::Session {
        id: SessionId::new(),
        user_id: admin.id,
        client: None,
        device: None,
        device_id: None,
        app_version: None,
        remote_addr: None,
        created_at: now - time::Duration::seconds(jwt.max_session_lifetime_secs + 60),
        last_seen_at: now,
        expires_at: now + time::Duration::days(1),
        refresh_token_hash: Some(noflex_auth::hash_refresh_token("old-session")),
        locked_at: None,
    };
    store.create_session(&session).await.expect("create");

    let resp = schema
        .execute(refresh_mutation("old-session").as_str())
        .await;
    assert_eq!(
        refresh_code(&resp),
        Some("REFRESH_TOKEN_INVALID".into()),
        "{:?}",
        resp.errors
    );
    assert!(store.get_session(session.id).await.expect("get").is_none());
}

/// Rotation slides expiry by the refresh TTL but clamps it to the
/// absolute lifetime.
#[tokio::test]
async fn rotation_never_extends_past_the_absolute_lifetime() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema).await;
    let admin = store
        .get_user_by_username("admin")
        .await
        .expect("get admin")
        .expect("admin");
    let now = time::OffsetDateTime::now_utc();
    let created_at = now - time::Duration::seconds(jwt.max_session_lifetime_secs - 3600);
    let session = noflex_storage::domain::Session {
        id: SessionId::new(),
        user_id: admin.id,
        client: None,
        device: None,
        device_id: None,
        app_version: None,
        remote_addr: None,
        created_at,
        last_seen_at: now,
        expires_at: now + time::Duration::days(1),
        refresh_token_hash: Some(noflex_auth::hash_refresh_token("aging-session")),
        locked_at: None,
    };
    store.create_session(&session).await.expect("create");

    let resp = schema
        .execute(refresh_mutation("aging-session").as_str())
        .await;
    assert!(resp.errors.is_empty(), "refresh: {:?}", resp.errors);
    let after = store
        .get_session(session.id)
        .await
        .expect("get")
        .expect("session");
    assert_eq!(
        after.expires_at,
        created_at + time::Duration::seconds(jwt.max_session_lifetime_secs)
    );
}

/// Sign-in records the transport's client address on the session row.
#[tokio::test]
async fn login_records_the_client_address() {
    let (schema, jwt, store) = authed_schema().await;
    setup_admin(&schema).await;
    let resp = schema
        .execute(
            Request::new(r#"mutation { login(username: "admin", password: "hunter2") { token } }"#)
                .data(noflex_auth::ClientAddr("198.51.100.7".parse().unwrap())),
        )
        .await;
    assert!(resp.errors.is_empty(), "login: {:?}", resp.errors);
    let token = resp.data.into_json().unwrap()["login"]["token"]
        .as_str()
        .unwrap()
        .to_string();
    let session = store
        .get_session(session_of(&jwt, &token))
        .await
        .expect("get")
        .expect("session");
    assert_eq!(session.remote_addr.as_deref(), Some("198.51.100.7"));
}
