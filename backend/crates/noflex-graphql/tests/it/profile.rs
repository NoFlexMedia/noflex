// SPDX-License-Identifier: AGPL-3.0-only
//
// A viewer's name is theirs to change, but their username is a credential: it takes
// a password, and everything keyed on the account has to survive it. Their PIN is
// one too, of a lesser kind: it only opens a session a shared device already holds.

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
        has_pin: false,
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

/// Sign `viewer` in as a device would, returning its refresh token.
async fn sign_in(fixture: &Fixture) -> String {
    let response = fixture
        .schema
        .execute(r#"mutation { login(username: "viewer", password: "hunter22") { refreshToken } }"#)
        .await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    response.data.into_json().expect("json")["login"]["refreshToken"]
        .as_str()
        .expect("refresh token")
        .to_string()
}

fn set_pin(pin: &str, password: &str) -> String {
    format!(r#"mutation {{ setPin(pin: "{pin}", currentPassword: "{password}") {{ hasPin }} }}"#)
}

fn lock(refresh_token: &str) -> String {
    format!(r#"mutation {{ lockSession(refreshToken: "{refresh_token}") }}"#)
}

fn unlock(refresh_token: &str, pin: &str) -> String {
    format!(
        r#"mutation {{ unlockSession(refreshToken: "{refresh_token}", pin: "{pin}") {{ refreshToken }} }}"#
    )
}

fn refresh(refresh_token: &str) -> String {
    format!(r#"mutation {{ refreshSession(refreshToken: "{refresh_token}") {{ refreshToken }} }}"#)
}

/// The `extensions` of a response's one error.
fn refusal(response: &async_graphql::Response) -> serde_json::Value {
    assert_eq!(response.errors.len(), 1, "{:?}", response.errors);
    serde_json::to_value(&response.errors[0].extensions).expect("extensions")
}

#[tokio::test]
async fn a_pin_is_a_few_digits_chosen_with_the_password() {
    let fixture = fixture().await;
    for shapeless in ["123", "1234567", "12ab", "١٢٣٤"] {
        let refused = run_as(&fixture, &fixture.user, set_pin(shapeless, "hunter22")).await;
        assert_eq!(refused.errors[0].message, "a PIN is 4 to 6 digits");
    }
    let wrong = run_as(&fixture, &fixture.user, set_pin("1234", "wrong")).await;
    assert_eq!(wrong.errors[0].message, "incorrect current password");
    let seen = field(&fixture, "{ viewer { hasPin } }", "viewer").await;
    assert_eq!(seen["hasPin"], false);

    let set = field(&fixture, &set_pin("1234", "hunter22"), "setPin").await;
    assert_eq!(set["hasPin"], true);
}

#[tokio::test]
async fn a_locked_session_opens_only_with_its_pin() {
    let fixture = fixture().await;
    let token = sign_in(&fixture).await;
    let execute = |query: String| fixture.schema.execute(query);

    let unlockable = execute(lock(&token)).await;
    assert_eq!(
        unlockable.data.into_json().expect("json")["lockSession"],
        false,
        "without a PIN there's nothing to lock with"
    );
    field(&fixture, &set_pin("1234", "hunter22"), "setPin").await;
    let locked = execute(lock(&token)).await;
    assert_eq!(locked.data.into_json().expect("json")["lockSession"], true);

    let refused = execute(refresh(&token)).await;
    assert_eq!(refusal(&refused)["code"], "SESSION_LOCKED");

    // The lock screen still learns whose tile to draw, and which tiles are dead.
    let profiles = execute(format!(
        r#"{{ rememberedProfiles(refreshTokens: ["{token}", "not-a-token"]) {{ userId name hasPin }} }}"#
    ))
    .await;
    assert!(profiles.errors.is_empty(), "{:?}", profiles.errors);
    let profiles = profiles.data.into_json().expect("json")["rememberedProfiles"].clone();
    assert_eq!(profiles[0]["userId"], fixture.user.id.to_string());
    assert_eq!(profiles[0]["name"], "viewer");
    assert_eq!(profiles[0]["hasPin"], true);
    assert!(profiles[1].is_null());

    let wrong = execute(unlock(&token, "0000")).await;
    let wrong = refusal(&wrong);
    assert_eq!(wrong["code"], "PIN_INCORRECT");
    assert_eq!(wrong["attemptsLeft"], 4);

    let opened = execute(unlock(&token, "1234")).await;
    assert!(opened.errors.is_empty(), "{:?}", opened.errors);
    let rotated = opened.data.into_json().expect("json")["unlockSession"]["refreshToken"]
        .as_str()
        .expect("refresh token")
        .to_string();
    assert_ne!(rotated, token, "opening rotates, as a refresh does");
    let refreshed = execute(refresh(&rotated)).await;
    assert!(refreshed.errors.is_empty(), "{:?}", refreshed.errors);
}

#[tokio::test]
async fn five_wrong_pins_end_the_session() {
    let fixture = fixture().await;
    field(&fixture, &set_pin("1234", "hunter22"), "setPin").await;
    let token = sign_in(&fixture).await;

    for left in (1..=4).rev() {
        let wrong = fixture.schema.execute(unlock(&token, "0000")).await;
        assert_eq!(refusal(&wrong)["attemptsLeft"], left);
    }
    let last = fixture.schema.execute(unlock(&token, "0000")).await;
    assert_eq!(refusal(&last)["code"], "PIN_ATTEMPTS_EXHAUSTED");
    let too_late = fixture.schema.execute(unlock(&token, "1234")).await;
    assert_eq!(
        refusal(&too_late)["code"],
        "REFRESH_TOKEN_INVALID",
        "the right PIN is no use once the session is gone"
    );
}

#[tokio::test]
async fn removing_a_pin_opens_what_it_locked() {
    let fixture = fixture().await;
    field(&fixture, &set_pin("1234", "hunter22"), "setPin").await;
    let token = sign_in(&fixture).await;
    fixture.schema.execute(lock(&token)).await;

    let wrong = run_as(
        &fixture,
        &fixture.user,
        r#"mutation { removePin(currentPassword: "wrong") { hasPin } }"#.into(),
    )
    .await;
    assert_eq!(wrong.errors[0].message, "incorrect current password");
    let removed = field(
        &fixture,
        r#"mutation { removePin(currentPassword: "hunter22") { hasPin } }"#,
        "removePin",
    )
    .await;
    assert_eq!(removed["hasPin"], false);

    let refreshed = fixture.schema.execute(refresh(&token)).await;
    assert!(refreshed.errors.is_empty(), "{:?}", refreshed.errors);
}
