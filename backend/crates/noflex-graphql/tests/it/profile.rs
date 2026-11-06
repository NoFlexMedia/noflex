// SPDX-License-Identifier: AGPL-3.0-only
//
// A viewer's name is theirs to change, but their username is a credential: it takes
// a password, and everything keyed on the account has to survive it.

use std::sync::Arc;

use async_graphql::Request;
use noflex_auth::Principal;
use noflex_core::{UserId, UserRole};
use noflex_graphql::{NoflexSchema, SchemaContext, build_schema};
use noflex_storage::{RequestUser, Storage, domain::User};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

/// One white pixel.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==";

struct Fixture {
    schema: NoflexSchema,
    storage: Arc<dyn Storage>,
    user: User,
}

async fn account(storage: &dyn Storage, username: &str, password: &str) -> User {
    let user = User {
        id: UserId::new(),
        username: username.into(),
        display_name: None,
        password_hash: if password.is_empty() {
            String::new()
        } else {
            noflex_auth::hash_password(password).await.expect("hash")
        },
        is_admin: false,
        role: UserRole::User,
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
    };
    storage.upsert_user(&user).await.expect("user");
    user
}

async fn fixture() -> Fixture {
    let store = Arc::new(TursoStore::open_temp().await.expect("store"));
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = store;
    let schema = build_schema(SchemaContext {
        storage: Some(storage.clone()),
        jwt: Some(Arc::new(
            noflex_auth::JwtConfig::dev(b"test-secret-32-bytes-please-use-it").expect("jwt"),
        )),
        ..Default::default()
    });
    let user = account(storage.as_ref(), "viewer", "hunter22").await;
    Fixture {
        schema,
        storage,
        user,
    }
}

async fn run_as(fixture: &Fixture, user: &User, query: String) -> async_graphql::Response {
    fixture
        .schema
        .execute(
            Request::new(query)
                .data(Principal {
                    user_id: user.id,
                    is_admin: user.is_admin,
                    session_id: None,
                })
                .data(RequestUser {
                    user_id: user.id,
                    is_admin: user.is_admin,
                }),
        )
        .await
}

async fn field(fixture: &Fixture, query: &str, path: &str) -> serde_json::Value {
    let response = run_as(fixture, &fixture.user, query.to_string()).await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    response.data.into_json().expect("json")[path].clone()
}

#[tokio::test]
async fn a_name_stands_in_for_the_username_until_its_cleared() {
    let fixture = fixture().await;
    let viewer = "{ viewer { name username } }";
    let seen = field(&fixture, viewer, "viewer").await;
    assert_eq!(seen["name"], "viewer", "nothing chosen yet");

    let named = field(
        &fixture,
        r#"mutation { updateProfile(name: "  Vera Viewer ") { name username } }"#,
        "updateProfile",
    )
    .await;
    assert_eq!(named["name"], "Vera Viewer");
    assert_eq!(named["username"], "viewer");

    let cleared = field(
        &fixture,
        r#"mutation { updateProfile(name: "   ") { name } }"#,
        "updateProfile",
    )
    .await;
    assert_eq!(cleared["name"], "viewer");
}

#[tokio::test]
async fn changing_a_username_takes_the_password_and_moves_the_sign_in() {
    let fixture = fixture().await;
    account(fixture.storage.as_ref(), "taken", "hunter22").await;
    let change = |username: &str, password: &str| {
        format!(
            r#"mutation {{ changeUsername(username: "{username}", currentPassword: "{password}") {{ username }} }}"#
        )
    };

    let wrong = run_as(&fixture, &fixture.user, change("vera", "wrong")).await;
    assert_eq!(wrong.errors[0].message, "incorrect current password");
    let taken = run_as(&fixture, &fixture.user, change("taken", "hunter22")).await;
    assert_eq!(taken.errors[0].message, "that username is taken");
    let blank = run_as(&fixture, &fixture.user, change("  ", "hunter22")).await;
    assert!(!blank.errors.is_empty());

    let changed = run_as(&fixture, &fixture.user, change("vera", "hunter22")).await;
    assert!(changed.errors.is_empty(), "{:?}", changed.errors);

    let login = |username: &str| {
        format!(r#"mutation {{ login(username: "{username}", password: "hunter22") {{ token }} }}"#)
    };
    assert!(
        !fixture
            .schema
            .execute(login("viewer"))
            .await
            .errors
            .is_empty(),
        "the old username is gone"
    );
    assert!(
        fixture
            .schema
            .execute(login("vera"))
            .await
            .errors
            .is_empty(),
        "and the new one signs in"
    );
    let still = field(&fixture, "{ viewer { id username } }", "viewer").await;
    assert_eq!(still["id"], fixture.user.id.to_string(), "the same account");
}

#[tokio::test]
async fn an_account_with_no_password_has_none_to_give() {
    let fixture = fixture().await;
    let provider_only = account(fixture.storage.as_ref(), "from-sso", "").await;
    let changed = run_as(
        &fixture,
        &provider_only,
        r#"mutation { changeUsername(username: "vera", currentPassword: "") { username } }"#.into(),
    )
    .await;
    assert!(changed.errors.is_empty(), "{:?}", changed.errors);
}

#[tokio::test]
async fn a_picture_gets_a_url_that_changes_with_it_and_can_be_removed() {
    let fixture = fixture().await;
    let refused = run_as(
        &fixture,
        &fixture.user,
        r#"mutation { setProfilePicture(image: "bm90IGEgcGljdHVyZQ==") { id } }"#.into(),
    )
    .await;
    assert!(!refused.errors.is_empty(), "text isn't a picture");
    assert_eq!(
        field(&fixture, "{ viewer { avatarUrl } }", "viewer").await["avatarUrl"],
        serde_json::Value::Null
    );

    let set = field(
        &fixture,
        &format!(r#"mutation {{ setProfilePicture(image: "{PNG}") {{ avatarUrl }} }}"#),
        "setProfilePicture",
    )
    .await;
    let url = set["avatarUrl"].as_str().expect("url");
    assert!(
        url.starts_with(&format!("/avatars/{}?v=", fixture.user.id)),
        "{url}"
    );
    let stored = fixture
        .storage
        .user_avatar(fixture.user.id)
        .await
        .expect("get")
        .expect("picture");
    assert_eq!(
        &stored[..2],
        [0xFF, 0xD8],
        "served as JPEG, whatever came in"
    );

    let removed = field(
        &fixture,
        "mutation { removeProfilePicture { avatarUrl } }",
        "removeProfilePicture",
    )
    .await;
    assert_eq!(removed["avatarUrl"], serde_json::Value::Null);
}
