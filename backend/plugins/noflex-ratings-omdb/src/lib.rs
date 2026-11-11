// SPDX-License-Identifier: AGPL-3.0-only
//
// OMDb ratings provider — first-party WASM plugin. It exports `ratings`
// alone: OMDb's own records are IMDb's, so another provider describes the
// work and this adds what OMDb has of IMDb, Rotten Tomatoes and Metacritic.
//
// A lookup needs an IMDb id and an administrator's key (the `api_key`
// secret); a work without the one gets no ratings, and without the other
// every call is `unavailable`. Films and series are looked up, episodes only
// with the `episodes` setting, as each costs one of a free key's 1,000
// lookups a day. A spent allowance is answered `rate-limited` for an hour
// without asking OMDb again. Parsing lives in `api` so it is testable
// off-target; this module is only the WIT glue.

mod api;

#[cfg(target_arch = "wasm32")]
mod guest {
    // The canonical `ratings-plugin` world plus WASI 0.3's monotonic clock,
    // which times the backoff after a spent allowance.
    wit_bindgen::generate!({
        inline: r#"
            package noflex:omdb;

            world omdb {
                include noflex:plugin/ratings-plugin@0.2.0;
                import wasi:clocks/monotonic-clock@0.3.0;
            }
        "#,
        path: "../../crates/noflex-plugin-wit/wit",
        world: "noflex:omdb/omdb",
        generate_all,
    });

    /// Embedded plugin manifest, preserved as a custom section in the
    /// final component so the host can read it without a post-build step.
    #[unsafe(link_section = "noflex:manifest")]
    #[used]
    static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
        *include_bytes!("manifest.toml");

    use std::cell::{Cell, RefCell};

    use exports::noflex::plugin::{background, ratings};
    use noflex::plugin::{
        host_config, host_net, host_secrets,
        types::{ExternalId, ExternalIdKind, PluginError, Rating, WorkKind},
    };

    use crate::api::{self, Answer};

    const TIMEOUT_MS: u32 = 15_000;
    const SECRET_KEY: &str = "api_key";
    const NANOS_PER_SEC: u64 = 1_000_000_000;

    thread_local! {
        /// Read once: every secret read is audit-logged host-side, and a
        /// change of key reloads the plugin.
        static API_KEY: RefCell<Option<String>> = const { RefCell::new(None) };
        static EPISODES: Cell<Option<bool>> = const { Cell::new(None) };
        /// The monotonic mark before which the allowance is spent.
        static LIMITED_UNTIL: Cell<u64> = const { Cell::new(0) };
    }

    fn now() -> u64 {
        wasi::clocks::monotonic_clock::now()
    }

    async fn api_key() -> Result<String, PluginError> {
        if let Some(key) = API_KEY.with(|k| k.borrow().clone()) {
            return Ok(key);
        }
        let key = match host_secrets::read(SECRET_KEY.into()).await {
            Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
            Ok(_) | Err(PluginError::Unavailable(_)) => {
                return Err(PluginError::Unavailable(
                    "omdb: no API key; set one in the OMDb plugin's settings".into(),
                ));
            }
            Err(e) => {
                return Err(PluginError::Unavailable(format!(
                    "omdb: secret `{SECRET_KEY}` unreadable: {e:?}"
                )));
            }
        };
        API_KEY.with(|k| *k.borrow_mut() = Some(key.clone()));
        Ok(key)
    }

    fn rates_episodes() -> bool {
        if let Some(episodes) = EPISODES.get() {
            return episodes;
        }
        let episodes = host_config::read()
            .parse::<toml::Table>()
            .ok()
            .and_then(|config| config.get("episodes").and_then(toml::Value::as_bool))
            .unwrap_or(false);
        EPISODES.set(Some(episodes));
        episodes
    }

    fn limited_for() -> Option<u32> {
        let left = LIMITED_UNTIL.get().saturating_sub(now());
        (left > 0).then(|| left.div_ceil(NANOS_PER_SEC).try_into().unwrap_or(u32::MAX))
    }

    async fn lookup(url: String) -> Result<Answer, PluginError> {
        let response = host_net::request(host_net::HttpRequest {
            method: "GET".into(),
            url,
            headers: vec![
                ("accept".into(), "application/json".into()),
                ("user-agent".into(), api::USER_AGENT.into()),
            ],
            body: None,
            timeout_ms: Some(TIMEOUT_MS),
        })
        .await
        .map_err(|e| PluginError::Unavailable(format!("omdb request failed: {e:?}")))?;
        // A refusal is JSON under 200 or 401 alike; anything else that isn't
        // is OMDb having trouble.
        match api::parse(&response.body) {
            Ok(answer) => Ok(answer),
            Err(error) if response.status == 200 => Err(PluginError::Internal(error)),
            Err(_) => Err(PluginError::Unavailable(format!(
                "omdb answered {}",
                response.status
            ))),
        }
    }

    struct Omdb;

    impl ratings::Guest for Omdb {
        fn attribution() -> Option<ratings::ProviderAttribution> {
            Some(ratings::ProviderAttribution {
                // OMDb licenses its data CC BY-NC 4.0, which asks for credit
                // and a link to the licence.
                text: "Ratings from OMDb, the Open Movie Database, licensed CC BY-NC 4.0 \
                       (creativecommons.org/licenses/by-nc/4.0)."
                    .into(),
                url: Some("https://www.omdbapi.com/".into()),
                logo: None,
            })
        }

        async fn fetch(ids: Vec<ExternalId>, kind: WorkKind) -> Result<Vec<Rating>, PluginError> {
            let rated = match kind {
                WorkKind::Movie | WorkKind::Series => true,
                WorkKind::Episode => rates_episodes(),
                _ => false,
            };
            let imdb_id = ids.iter().find(|id| id.kind == ExternalIdKind::Imdb);
            let Some(imdb_id) = imdb_id.filter(|_| rated) else {
                return Ok(Vec::new());
            };
            if let Some(secs) = limited_for() {
                return Err(PluginError::RateLimited(secs));
            }
            let key = api_key().await?;
            let Some(url) = api::lookup_url(&imdb_id.value, &key) else {
                return Ok(Vec::new());
            };
            match lookup(url).await? {
                Answer::Ratings(found) => Ok(found
                    .into_iter()
                    .map(|r| Rating {
                        source: r.source,
                        value: r.value,
                        scale: r.scale,
                        votes: r.votes,
                    })
                    .collect()),
                Answer::NotFound => Ok(Vec::new()),
                Answer::KeyRefused(error) => {
                    API_KEY.with(|k| *k.borrow_mut() = None);
                    Err(PluginError::Unavailable(format!(
                        "omdb refused the API key: {error}"
                    )))
                }
                Answer::LimitReached(_) => {
                    let secs = api::LIMIT_BACKOFF_SECS;
                    LIMITED_UNTIL.set(now().saturating_add(u64::from(secs) * NANOS_PER_SEC));
                    Err(PluginError::RateLimited(secs))
                }
                Answer::Refused(error) => Err(PluginError::Unavailable(format!("omdb: {error}"))),
            }
        }
    }

    impl background::Guest for Omdb {
        async fn run_task(
            task: String,
            _cursor: Option<String>,
            _payload: Option<String>,
        ) -> Result<background::TaskOutcome, background::PluginError> {
            Err(background::PluginError::BadInput(format!(
                "omdb: unknown task `{task}`"
            )))
        }
    }

    export!(Omdb);
}
