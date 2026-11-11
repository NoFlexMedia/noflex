// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for the library write imports. `link-complete` acts
// for its user, so its `callback_query` is a command channel: the guest runs
// one import and reports the outcome as a single `result` secret, which the
// host tests read straight off the call. Commands, `:`-separated:
//
//   find:<kind>:<value>                     -> work ids, comma-separated
//   episode:<kind>:<value>:<season>:<ep>    -> a work id, or `none`
//   tags:<work>:<namespace>:<slug,...>      -> `ok`, or `err:<kind>`
//   collection:<key>:<work,...>             -> `ok:<collections>`
//   import:<user>:<work>:<played-ms>:<rating>:<rated-ms>
//                                           -> `ok:<changed>`
//   history:<user>:<since-ms or ->          -> `<work>=<played-ms>/<rating>;...`
//
// The `per-user` task writes `ticked` for whichever user it runs bound to.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    inline: r#"
        package noflex:fixture-library-writes;

        world fixture {
            include noflex:plugin/user-link-plugin@0.2.0;
            import noflex:plugin/host-tags@0.2.0;
            import noflex:plugin/host-collections@0.2.0;
            import noflex:plugin/host-user-state@0.2.0;
        }
    "#,
    path: "../../../../noflex-plugin-wit/wit",
    world: "noflex:fixture-library-writes/fixture",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{background, user_link};
use noflex::plugin::{
    host_collections, host_library, host_secrets, host_tags, host_user_state,
    types::{ExternalId, ExternalIdKind, PluginError},
};

struct Fixture;

fn kind(name: &str) -> ExternalIdKind {
    match name {
        "tmdb" => ExternalIdKind::Tmdb,
        "tvdb" => ExternalIdKind::Tvdb,
        "imdb" => ExternalIdKind::Imdb,
        _ => ExternalIdKind::Custom,
    }
}

fn error_kind(error: &PluginError) -> &'static str {
    match error {
        PluginError::CapabilityDenied(_) => "capability-denied",
        PluginError::Unavailable(_) => "unavailable",
        PluginError::BadInput(_) => "bad-input",
        PluginError::NotFound => "not-found",
        _ => "other",
    }
}

fn number<T: std::str::FromStr>(raw: &str) -> Option<T> {
    raw.parse().ok()
}

async fn run(command: &str) -> String {
    let parts = command.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        ["find", id_kind, value] => host_library::find_works(ExternalId {
            kind: kind(id_kind),
            value: value.to_string(),
        })
        .await
        .into_iter()
        .map(|work| work.id)
        .collect::<Vec<_>>()
        .join(","),
        ["episode", id_kind, value, season, episode] => {
            let ids = vec![ExternalId {
                kind: kind(id_kind),
                value: value.to_string(),
            }];
            let (Some(season), Some(episode)) = (number(season), number(episode)) else {
                return "bad command".into();
            };
            host_library::find_episode(ids, season, episode)
                .await
                .map_or("none".into(), |work| work.id)
        }
        ["tags", work, namespace, slugs] => {
            let tags = slugs
                .split(',')
                .filter(|slug| !slug.is_empty())
                .map(|slug| host_tags::TagDraft {
                    slug: slug.to_string(),
                    display: slug.to_uppercase(),
                    weight: 1.0,
                })
                .collect();
            match host_tags::set_tags(work.to_string(), namespace.to_string(), tags).await {
                Ok(()) => "ok".into(),
                Err(e) => format!("err:{}", error_kind(&e)),
            }
        }
        ["collection", key, works] => {
            let works = works
                .split(',')
                .filter(|work| !work.is_empty())
                .map(str::to_string)
                .collect();
            match host_collections::sync_collection(
                key.to_string(),
                format!("List {key}"),
                None,
                works,
            )
            .await
            {
                Ok(count) => format!("ok:{count}"),
                Err(e) => format!("err:{}", error_kind(&e)),
            }
        }
        ["import", user, work, played, rating, rated] => {
            let item = host_user_state::StateImport {
                work: work.to_string(),
                played_at: number(played),
                play_count: None,
                rating: number(rating).zip(number(rated)).map(|(value, rated_at)| {
                    host_user_state::UserRating { value, rated_at }
                }),
                favorite: false,
            };
            match host_user_state::import_user_state(user.to_string(), vec![item]).await {
                Ok(changed) => format!("ok:{changed}"),
                Err(e) => format!("err:{}", error_kind(&e)),
            }
        }
        ["history", user, since] => {
            match host_user_state::user_history(user.to_string(), number(since), None).await {
                Ok(page) => page
                    .entries
                    .iter()
                    .map(|entry| {
                        let played = entry
                            .played_at
                            .map_or("-".to_string(), |at| at.to_string());
                        let rating = entry
                            .rating
                            .as_ref()
                            .map_or("-".to_string(), |rating| rating.value.to_string());
                        format!("{}={played}/{rating}", entry.work)
                    })
                    .collect::<Vec<_>>()
                    .join(";"),
                Err(e) => format!("err:{}", error_kind(&e)),
            }
        }
        _ => "bad command".into(),
    }
}

impl user_link::Guest for Fixture {
    async fn link_start(user: String) -> Result<user_link::LinkChallenge, PluginError> {
        Ok(user_link::LinkChallenge {
            auth_url: format!("https://example.invalid/authorize?user={user}"),
            state: "fixture".into(),
            user_code: None,
        })
    }

    async fn link_complete(
        _user: String,
        _state: String,
        callback_query: String,
    ) -> Result<Vec<user_link::UserSecret>, PluginError> {
        Ok(vec![user_link::UserSecret {
            key: "result".into(),
            value: run(&callback_query).await,
        }])
    }

    async fn unlink(_user: String) -> Result<(), PluginError> {
        Ok(())
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        _task: String,
        _cursor: Option<String>,
        payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        // A per-user tick names its user as the payload.
        if let Some(user) = payload {
            host_secrets::write_for_user(user, "ticked".into(), "yes".into()).await?;
        }
        Ok(background::TaskOutcome::Done)
    }
}

export!(Fixture);
