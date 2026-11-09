// SPDX-License-Identifier: AGPL-3.0-only
//
// Multiple servers, each a separate schema and store here: invites, linking,
// and carrying watch state between them the way clients do.

use std::sync::Arc;

use async_graphql::{Request, Variables};
use noflex_auth::Principal;
use noflex_core::{
    ExternalId, ExternalIdKind, LibraryId, MetadataStatus, PlayTarget, UserId, WorkId, WorkKind,
};
use noflex_graphql::{NoflexSchema, SchemaContext, build_schema};
use noflex_storage::{
    RequestUser,
    domain::{Library, LibraryConfig, LibraryKind, Work},
    store::Storage,
};
use noflex_storage_turso::TursoStore;
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};

const TEST_JWT_SECRET: &[u8] = b"test-secret-32-bytes-please-use-it";

struct Server {
    schema: NoflexSchema,
    store: Arc<TursoStore>,
    admin: Principal,
    library: LibraryId,
}

impl Server {
    async fn start() -> Self {
        let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
        store.migrate().await.expect("migrate");
        let schema = build_schema(SchemaContext {
            storage: Some(store.clone()),
            jwt: Some(Arc::new(
                noflex_auth::JwtConfig::dev(TEST_JWT_SECRET).expect("JWT config"),
            )),
            ..Default::default()
        });
        let response = schema
            .execute(
                r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { userId } }"#,
            )
            .await;
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        let user = response.data.into_json().unwrap()["completeSetupWizard"]["userId"]
            .as_str()
            .expect("userId")
            .parse()
            .expect("uuid");
        let library = Library {
            id: LibraryId::new(),
            name: "Media".to_string(),
            kind: LibraryKind::Mixed,
            paths: vec!["/srv/media".to_string()],
            config: LibraryConfig::default(),
        };
        store.upsert_library(&library).await.expect("library");
        Self {
            schema,
            store,
            admin: Principal {
                user_id: UserId::from_uuid(user),
                is_admin: true,
                session_id: None,
            },
            library: library.id,
        }
    }

    async fn run(&self, principal: Option<Principal>, query: &str, variables: Value) -> Value {
        let result = self.try_run(principal, query, variables).await;
        result.unwrap_or_else(|errors| panic!("{query}: {errors}"))
    }

    async fn try_run(
        &self,
        principal: Option<Principal>,
        query: &str,
        variables: Value,
    ) -> Result<Value, String> {
        let mut request = Request::new(query).variables(Variables::from_json(variables));
        if let Some(principal) = principal {
            request = request.data(principal).data(RequestUser {
                user_id: principal.user_id,
                is_admin: principal.is_admin,
            });
        }
        let response = self.schema.execute(request).await;
        if response.errors.is_empty() {
            Ok(response.data.into_json().unwrap())
        } else {
            Err(response
                .errors
                .iter()
                .map(|error| error.message.clone())
                .collect::<Vec<_>>()
                .join("; "))
        }
    }

    async fn work(
        &self,
        kind: WorkKind,
        title: &str,
        parent: Option<WorkId>,
        external_ids: Vec<ExternalId>,
        season: Option<i32>,
        episode: Option<i32>,
    ) -> WorkId {
        let now = OffsetDateTime::now_utc();
        let work = Work {
            id: WorkId::new(),
            kind,
            parent_id: parent,
            library_id: self.library,
            title: title.to_string(),
            sort_title: title.to_lowercase(),
            year: None,
            release_date: None,
            overview: None,
            external_ids,
            index_number: episode,
            index_end: None,
            parent_index: season,
            metadata_status: MetadataStatus::Unconfirmed,
            tagline: None,
            original_title: None,
            community_rating: None,
            locked_fields: vec![],
            metadata_provider: None,
            added_at: now,
            updated_at: now,
        };
        self.store.upsert_work(&work).await.expect("work");
        work.id
    }

    /// A series the scanner's way, with one episode.
    async fn episode(&self, tmdb: &str, season: i32, episode: i32) -> WorkId {
        let series = self
            .work(
                WorkKind::Series,
                "Show",
                None,
                vec![tmdb_id(tmdb)],
                None,
                None,
            )
            .await;
        let season_work = self
            .work(
                WorkKind::Season,
                "Season",
                Some(series),
                vec![],
                Some(season),
                None,
            )
            .await;
        self.work(
            WorkKind::Episode,
            "Episode",
            Some(season_work),
            vec![],
            Some(season),
            Some(episode),
        )
        .await
    }

    async fn sync_items(&self, since: OffsetDateTime) -> Value {
        self.run(
            Some(self.admin),
            r#"query($since: DateTime!) { watchSyncItems(since: $since) {
                externalIds { kind value } season episode played positionMs lastPlayed
            } }"#,
            json!({ "since": since.format(&Rfc3339).unwrap() }),
        )
        .await["watchSyncItems"]
            .clone()
    }

    async fn apply(&self, items: Value) -> u64 {
        self.run(
            Some(self.admin),
            r#"mutation($items: [WatchSyncItemInput!]!) { syncWatchState(items: $items) }"#,
            json!({ "items": items }),
        )
        .await["syncWatchState"]
            .as_u64()
            .expect("count")
    }
}

fn tmdb_id(value: &str) -> ExternalId {
    ExternalId {
        kind: ExternalIdKind::Tmdb,
        value: value.to_string(),
    }
}

fn long_ago() -> OffsetDateTime {
    OffsetDateTime::now_utc() - Duration::days(30)
}

#[tokio::test]
async fn watching_on_one_server_carries_to_the_same_title_on_another() {
    let (home, friend) = (Server::start().await, Server::start().await);
    let movie_id = tmdb_id("movie/603");
    let at_home = home
        .work(
            WorkKind::Movie,
            "Movie",
            None,
            vec![movie_id.clone()],
            None,
            None,
        )
        .await;
    let at_friend = friend
        .work(
            WorkKind::Movie,
            "Same movie",
            None,
            vec![movie_id],
            None,
            None,
        )
        .await;
    home.store
        .mark_played(home.admin.user_id, PlayTarget::Work(at_home), true)
        .await
        .expect("played");

    let items = home.sync_items(long_ago()).await;
    assert_eq!(friend.apply(items.clone()).await, 1);
    let there = friend
        .store
        .user_data(friend.admin.user_id, PlayTarget::Work(at_friend))
        .await
        .expect("data");
    assert!(there.played);
    let here = home
        .store
        .user_data(home.admin.user_id, PlayTarget::Work(at_home))
        .await
        .expect("data");
    assert_eq!(
        there.last_played.map(|at| at.unix_timestamp()),
        here.last_played.map(|at| at.unix_timestamp()),
        "the original time travels with it"
    );

    assert_eq!(friend.apply(items).await, 0, "replaying changes nothing");
}

#[tokio::test]
async fn the_most_recent_activity_wins() {
    let (home, friend) = (Server::start().await, Server::start().await);
    let movie_id = tmdb_id("movie/1");
    let at_home = home
        .work(
            WorkKind::Movie,
            "Movie",
            None,
            vec![movie_id.clone()],
            None,
            None,
        )
        .await;
    let at_friend = friend
        .work(WorkKind::Movie, "Movie", None, vec![movie_id], None, None)
        .await;
    home.store
        .record_playback_progress(home.admin.user_id, PlayTarget::Work(at_home), 1_000)
        .await
        .expect("progress");
    let older = home.sync_items(long_ago()).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    friend
        .store
        .record_playback_progress(friend.admin.user_id, PlayTarget::Work(at_friend), 9_000)
        .await
        .expect("progress");

    assert_eq!(friend.apply(older).await, 0, "the friend's is newer");
    assert_eq!(
        friend
            .store
            .user_data(friend.admin.user_id, PlayTarget::Work(at_friend))
            .await
            .expect("data")
            .position_ms,
        Some(9_000)
    );
    assert_eq!(home.apply(friend.sync_items(long_ago()).await).await, 1);
    assert_eq!(
        home.store
            .user_data(home.admin.user_id, PlayTarget::Work(at_home))
            .await
            .expect("data")
            .position_ms,
        Some(9_000)
    );
}

#[tokio::test]
async fn episodes_match_by_series_and_number() {
    let (home, friend) = (Server::start().await, Server::start().await);
    let at_home = home.episode("tv/42", 3, 6).await;
    let at_friend = friend.episode("tv/42", 3, 6).await;
    let different_show = friend.episode("tv/99", 3, 6).await;
    home.store
        .mark_played(home.admin.user_id, PlayTarget::Work(at_home), true)
        .await
        .expect("played");

    let items = home.sync_items(long_ago()).await;
    assert_eq!(items[0]["season"], 3);
    assert_eq!(items[0]["episode"], 6);
    assert_eq!(items[0]["externalIds"][0]["value"], "tv/42");
    assert_eq!(friend.apply(items).await, 1);

    let played = |work| {
        let store = friend.store.clone();
        let user = friend.admin.user_id;
        async move {
            store
                .user_data(user, PlayTarget::Work(work))
                .await
                .expect("data")
                .played
        }
    };
    assert!(played(at_friend).await);
    assert!(!played(different_show).await);
}

#[tokio::test]
async fn an_episode_names_its_series_but_never_local_ids() {
    let server = Server::start().await;
    let scanner_id = ExternalId {
        kind: ExternalIdKind::Custom,
        value: "scan:same-key".to_string(),
    };
    let series = server
        .work(
            WorkKind::Series,
            "Show",
            None,
            vec![tmdb_id("tv/42"), scanner_id],
            None,
            None,
        )
        .await;
    let season = server
        .work(
            WorkKind::Season,
            "Season",
            Some(series),
            vec![],
            Some(3),
            None,
        )
        .await;
    let episode = server
        .work(
            WorkKind::Episode,
            "Episode",
            Some(season),
            vec![],
            Some(3),
            Some(7),
        )
        .await;
    let series_ids = |work: WorkId| {
        let server = &server;
        async move {
            server
                .run(
                    Some(server.admin),
                    r#"query($id: ID!) { work(id: $id) { seriesTitle seriesExternalIds { kind value } } }"#,
                    json!({ "id": work.to_string() }),
                )
                .await["work"]
                .clone()
        }
    };

    assert_eq!(
        series_ids(episode).await,
        json!({
            "seriesTitle": "Show",
            "seriesExternalIds": [{ "kind": "TMDB", "value": "tv/42" }],
        })
    );
    assert_eq!(
        series_ids(series).await,
        json!({ "seriesTitle": null, "seriesExternalIds": [] })
    );
}

#[tokio::test]
async fn only_provider_ids_ever_match() {
    let (home, friend) = (Server::start().await, Server::start().await);
    let scanner_id = ExternalId {
        kind: ExternalIdKind::Custom,
        value: "scan:same-key".to_string(),
    };
    let at_home = home
        .work(
            WorkKind::Movie,
            "Movie",
            None,
            vec![scanner_id.clone()],
            None,
            None,
        )
        .await;
    let at_friend = friend
        .work(
            WorkKind::Movie,
            "Unrelated",
            None,
            vec![scanner_id.clone()],
            None,
            None,
        )
        .await;
    home.store
        .mark_played(home.admin.user_id, PlayTarget::Work(at_home), true)
        .await
        .expect("played");

    assert_eq!(
        home.sync_items(long_ago()).await,
        json!([]),
        "unidentified titles aren't offered"
    );

    // Nor accepted, whatever a client sends.
    let forged = json!([{
        "externalIds": [{ "kind": "CUSTOM", "value": "scan:same-key" }],
        "played": true,
        "lastPlayed": OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),
    }]);
    assert_eq!(friend.apply(forged).await, 0);
    assert!(
        !friend
            .store
            .user_data(friend.admin.user_id, PlayTarget::Work(at_friend))
            .await
            .expect("data")
            .played
    );
}

#[tokio::test]
async fn an_invite_makes_one_ordinary_account_with_the_granted_libraries() {
    let server = Server::start().await;
    let other_library = Library {
        id: LibraryId::new(),
        name: "Private".to_string(),
        kind: LibraryKind::Mixed,
        paths: vec!["/srv/private".to_string()],
        config: LibraryConfig::default(),
    };
    server
        .store
        .upsert_library(&other_library)
        .await
        .expect("library");

    let create = r#"mutation($libraries: [ID!]!) {
        createInvite(libraryIds: $libraries) { token invite { id maxUses } }
    }"#;
    let variables = json!({ "libraries": [server.library.to_string()] });
    let denied = Principal {
        user_id: UserId::new(),
        is_admin: false,
        session_id: None,
    };
    assert!(
        server
            .try_run(Some(denied), create, variables.clone())
            .await
            .is_err(),
        "only admins invite"
    );
    let created = server.run(Some(server.admin), create, variables).await;
    let token = created["createInvite"]["token"].as_str().expect("token");

    let redeem = r#"mutation($token: String!, $name: String!) {
        redeemInvite(token: $token, username: $name, password: "long-enough-1") {
            token user { id isAdmin }
        }
    }"#;
    let redeemed = server
        .run(None, redeem, json!({ "token": token, "name": "friend" }))
        .await;
    assert_eq!(redeemed["redeemInvite"]["user"]["isAdmin"], false);
    let friend = UserId::from_uuid(
        redeemed["redeemInvite"]["user"]["id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    );
    assert_eq!(
        server
            .store
            .list_user_libraries(friend)
            .await
            .expect("grants"),
        [server.library],
        "only what the invite granted"
    );

    let again = server
        .try_run(None, redeem, json!({ "token": token, "name": "second" }))
        .await;
    assert!(again.unwrap_err().contains("expired or already been used"));
}

#[tokio::test]
async fn a_taken_username_does_not_burn_the_invite() {
    let server = Server::start().await;
    let created = server
        .run(
            Some(server.admin),
            r#"mutation { createInvite(libraryIds: []) { token } }"#,
            json!({}),
        )
        .await;
    let token = created["createInvite"]["token"].as_str().expect("token");
    let redeem = r#"mutation($token: String!, $name: String!) {
        redeemInvite(token: $token, username: $name, password: "long-enough-1") { token }
    }"#;
    let taken = server
        .try_run(None, redeem, json!({ "token": token, "name": "admin" }))
        .await;
    assert!(taken.unwrap_err().contains("taken"));
    server
        .run(None, redeem, json!({ "token": token, "name": "friend" }))
        .await;
}

#[tokio::test]
async fn linked_servers_are_private_and_their_origins_validated() {
    let server = Server::start().await;
    let user = |id| Principal {
        user_id: id,
        is_admin: false,
        session_id: None,
    };
    let add = r#"mutation($origin: String!) {
        addLinkedServer(origin: $origin, name: "Friend", apiKey: "key") { id origin apiKey }
    }"#;
    assert!(
        server
            .try_run(
                Some(server.admin),
                add,
                json!({ "origin": "https://x.example; script-src *" })
            )
            .await
            .is_err()
    );
    let added = server
        .run(
            Some(server.admin),
            add,
            json!({ "origin": "HTTPS://Friend.Example/" }),
        )
        .await;
    assert_eq!(added["addLinkedServer"]["origin"], "https://friend.example");

    let list = r#"{ linkedServers { id apiKey } }"#;
    assert_eq!(
        server.run(Some(server.admin), list, json!({})).await["linkedServers"][0]["apiKey"],
        "key"
    );

    let stranger = noflex_storage::domain::User {
        id: UserId::new(),
        username: "stranger".to_string(),
        display_name: None,
        password_hash: "argon2id$placeholder".to_string(),
        is_admin: false,
        role: noflex_core::UserRole::User,
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
        has_pin: false,
    };
    server.store.upsert_user(&stranger).await.expect("user");
    assert_eq!(
        server.run(Some(user(stranger.id)), list, json!({})).await["linkedServers"],
        json!([]),
        "another user's keys are invisible"
    );
    assert_eq!(
        server
            .run(
                Some(user(stranger.id)),
                r#"mutation($id: ID!) { removeLinkedServer(id: $id) }"#,
                json!({ "id": added["addLinkedServer"]["id"] }),
            )
            .await["removeLinkedServer"],
        false
    );
}
