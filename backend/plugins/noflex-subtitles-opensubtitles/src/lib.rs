// SPDX-License-Identifier: AGPL-3.0-only
//
// OpenSubtitles.com subtitle search — first-party WASM plugin.
//
// All egress goes through `host-net` to OpenSubtitles' REST API v1 and the
// host its download links point at. Every request carries the operator's
// `api_key` and a User-Agent naming NoFlex; there is no shared key, so
// without one every call answers `unavailable`. With a `username` and
// `password` the plugin signs in once per instance, which raises the daily
// download quota to the account's and may move it to the VIP API host.
//
// A search goes by hash and ids first, so a match timed for this exact
// file is flagged, then by title when that finds nothing. Downloads ask
// the API for SubRip whatever was uploaded, then fetch the temporary link.
// Request building and parsing live in `api`, testable off-target.

// `pub` so the host build doesn't see the modules as dead code — on wasm
// they are consumed by `guest`, on the host only by their unit tests.
pub mod api;
pub mod pace;

#[cfg(target_arch = "wasm32")]
mod guest {
    // The canonical `subtitle-plugin` world plus WASI 0.3's monotonic
    // clock, whose async `wait-until` lets the pacer sleep without
    // blocking the instance's other calls.
    wit_bindgen::generate!({
        inline: r#"
            package noflex:opensubtitles;

            world opensubtitles {
                include noflex:plugin/subtitle-plugin@0.2.0;
                import wasi:clocks/monotonic-clock@0.3.0;
            }
        "#,
        path: "../../crates/noflex-plugin-wit/wit",
        world: "noflex:opensubtitles/opensubtitles",
        generate_all,
    });

    /// Embedded plugin manifest, preserved as a custom section in the
    /// final component so the host can read it without a post-build step.
    #[unsafe(link_section = "noflex:manifest")]
    #[used]
    static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
        *include_bytes!("manifest.toml");

    use std::{cell::RefCell, time::SystemTime};

    use exports::noflex::plugin::{
        background,
        subtitle::{self, SubtitleMatch, SubtitleQuery},
    };
    use noflex::plugin::{
        host_net, host_secrets,
        types::{ExternalIdKind, PluginError, WorkKind},
    };

    use crate::{
        api::{self, IdKind, Search},
        pace::Pacer,
    };

    const TIMEOUT_MS: u32 = 15_000;

    thread_local! {
        /// 5 requests a second, OpenSubtitles' limit; queue at most 10s.
        static PACER: Pacer = const { Pacer::new(200_000_000, 10_000_000_000) };
        /// Read once: every secret read is audit-logged host-side, and the
        /// host reloads the plugin when a setting changes.
        static API_KEY: RefCell<Option<String>> = const { RefCell::new(None) };
        /// The signed-in session, once a download needed one.
        static SESSION: RefCell<Option<api::Session>> = const { RefCell::new(None) };
    }

    fn now() -> u64 {
        wasi::clocks::monotonic_clock::now()
    }

    async fn paced() -> Result<(), PluginError> {
        let now = now();
        let slot = PACER
            .with(|pacer| pacer.reserve(now))
            .map_err(PluginError::RateLimited)?;
        if slot > now {
            wasi::clocks::monotonic_clock::wait_until(slot).await;
        }
        Ok(())
    }

    /// A secret the operator set, or `None` when no tier sets it.
    async fn secret(key: &str) -> Result<Option<String>, PluginError> {
        match host_secrets::read(key.into()).await {
            Ok(value) if !value.trim().is_empty() => Ok(Some(value.trim().to_string())),
            Ok(_) | Err(PluginError::Unavailable(_)) => Ok(None),
            Err(e) => Err(PluginError::Unavailable(format!(
                "opensubtitles: secret `{key}` unreadable: {e:?}"
            ))),
        }
    }

    async fn api_key() -> Result<String, PluginError> {
        if let Some(key) = API_KEY.with(|k| k.borrow().clone()) {
            return Ok(key);
        }
        let key = secret("api_key").await?.ok_or_else(|| {
            PluginError::Unavailable(
                "no API key is set; add one from opensubtitles.com in the plugin's settings".into(),
            )
        })?;
        API_KEY.with(|k| *k.borrow_mut() = Some(key.clone()));
        Ok(key)
    }

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn unix_now() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    /// One API request, paced. The answers the caller can act on are
    /// returned; the rest become plugin errors.
    async fn call(
        method: &str,
        url: String,
        body: Option<String>,
        token: Option<&str>,
    ) -> Result<host_net::HttpResponse, PluginError> {
        paced().await?;
        let key = api_key().await?;
        let response = host_net::request(host_net::HttpRequest {
            method: method.into(),
            url,
            headers: api::headers(&key, token),
            body: body.map(String::into_bytes),
            timeout_ms: Some(TIMEOUT_MS),
        })
        .await
        .map_err(|e| PluginError::Unavailable(format!("opensubtitles request failed: {e:?}")))?;
        match response.status {
            200 => Ok(response),
            401 | 403 => {
                API_KEY.with(|k| *k.borrow_mut() = None);
                SESSION.with(|s| *s.borrow_mut() = None);
                Err(PluginError::Unavailable(
                    "OpenSubtitles turned down the API key or sign-in".into(),
                ))
            }
            406 => Err(PluginError::RateLimited(
                api::quota_reset_in(&response.body, unix_now()).unwrap_or(3600),
            )),
            429 => {
                let retry_after = header(&response.headers, "retry-after")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(1);
                PACER.with(|pacer| pacer.back_off(now(), retry_after));
                Err(PluginError::RateLimited(retry_after))
            }
            404 | 410 => Err(PluginError::NotFound),
            status => Err(PluginError::Unavailable(format!(
                "opensubtitles answered {status}"
            ))),
        }
    }

    /// The session to download with: signed in when the operator gave an
    /// account, else none, which downloads on the API key's own quota.
    async fn session() -> Result<Option<api::Session>, PluginError> {
        if let Some(session) = SESSION.with(|s| s.borrow().clone()) {
            return Ok(Some(session));
        }
        let (Some(username), Some(password)) =
            (secret("username").await?, secret("password").await?)
        else {
            return Ok(None);
        };
        let response = call(
            "POST",
            format!("https://{}/api/v1/login", api::API_HOST),
            Some(api::login_body(&username, &password)),
            None,
        )
        .await?;
        let session = api::parse_login(&response.body).map_err(PluginError::Internal)?;
        SESSION.with(|s| *s.borrow_mut() = Some(session.clone()));
        Ok(Some(session))
    }

    fn id_kind(kind: ExternalIdKind) -> IdKind {
        match kind {
            ExternalIdKind::Imdb => IdKind::Imdb,
            ExternalIdKind::Tmdb => IdKind::Tmdb,
            _ => IdKind::Other,
        }
    }

    fn offer(found: api::Found) -> SubtitleMatch {
        let (language, locale) = api::language(&found.language);
        SubtitleMatch {
            handle: found.file_id.to_string(),
            language,
            locale,
            format: api::FORMAT.into(),
            release: found.release,
            hearing_impaired: found.hearing_impaired,
            forced: found.foreign_parts_only,
            hash_match: found.moviehash_match,
            download_count: found.download_count,
            score: found.rating,
        }
    }

    struct OpenSubtitles;

    impl subtitle::Guest for OpenSubtitles {
        async fn search(query: SubtitleQuery) -> Result<Vec<SubtitleMatch>, PluginError> {
            if !matches!(query.kind, WorkKind::Movie | WorkKind::Episode) {
                return Ok(Vec::new());
            }
            let host = SESSION
                .with(|s| s.borrow().as_ref().map(|s| s.host.clone()))
                .unwrap_or_else(|| api::API_HOST.to_string());
            let search = Search {
                episode: query.kind == WorkKind::Episode,
                moviehash: query.moviehash.as_deref(),
                title: &query.title,
                year: query.year,
                ids: query
                    .external_ids
                    .iter()
                    .map(|id| (id_kind(id.kind), id.value.as_str()))
                    .collect(),
                series_ids: query
                    .series_external_ids
                    .iter()
                    .map(|id| (id_kind(id.kind), id.value.as_str()))
                    .collect(),
                season: query.season,
                number: query.episode,
                languages: &query.languages,
            };
            for url in search.urls(&host) {
                let response = call("GET", url, None, None).await?;
                let found = api::parse_search(&response.body).map_err(PluginError::Internal)?;
                if !found.is_empty() {
                    return Ok(found.into_iter().map(offer).collect());
                }
            }
            Ok(Vec::new())
        }

        async fn download(
            handle: String,
        ) -> Result<(String, wit_bindgen::StreamReader<u8>), PluginError> {
            let file_id: u64 = handle.parse().map_err(|_| {
                PluginError::BadInput(format!("not an OpenSubtitles file id: {handle}"))
            })?;
            let session = session().await?;
            let host = session
                .as_ref()
                .map_or(api::API_HOST, |s| s.host.as_str())
                .to_string();
            let response = call(
                "POST",
                format!("https://{host}/api/v1/download"),
                Some(api::download_body(file_id)),
                session.as_ref().map(|s| s.token.as_str()),
            )
            .await?;
            let link = api::parse_download(&response.body).map_err(PluginError::Internal)?;
            // The link is a file, not the API: no key, no token, no pacing.
            let file = host_net::request(host_net::HttpRequest {
                method: "GET".into(),
                url: link.url,
                headers: vec![("user-agent".into(), api::USER_AGENT.into())],
                body: None,
                timeout_ms: Some(TIMEOUT_MS),
            })
            .await
            .map_err(|e| PluginError::Unavailable(format!("opensubtitles file failed: {e:?}")))?;
            if file.status != 200 {
                return Err(PluginError::Unavailable(format!(
                    "opensubtitles file answered {}",
                    file.status
                )));
            }
            let (mut writer, reader) = wit_stream::new::<u8>();
            // The host reads only after this returns, so write from a task.
            let bytes = file.body;
            wit_bindgen::spawn_local(async move {
                writer.write_all(bytes).await;
            });
            Ok((api::FORMAT.into(), reader))
        }
    }

    impl background::Guest for OpenSubtitles {
        async fn run_task(
            task: String,
            _cursor: Option<String>,
            _payload: Option<String>,
        ) -> Result<background::TaskOutcome, background::PluginError> {
            Err(background::PluginError::BadInput(format!(
                "opensubtitles: unknown task `{task}`"
            )))
        }
    }

    export!(OpenSubtitles);
}
