// SPDX-License-Identifier: AGPL-3.0-only
//
// TheTVDB metadata + image provider — first-party WASM plugin.
//
// All egress goes through `host-net` to the manifest's two hosts. `/login`
// trades NoFlex's shared key, or the `api_key` secret when an operator sets one
// (and `pin`, for a user-supported key), for a month-long bearer token, held
// until the API refuses it. Both secrets live in `tvdb.toml` in a secret tier.
// Request building and parsing live in `api` so they are testable off-target;
// this module is only the WIT glue.
//
// Season and episode matches need no request: the host's search hints name the
// parent series' id and the numbers, from which the child's id follows; `fetch`
// then fails for numbers TVDB does not have.

mod api;

#[cfg(target_arch = "wasm32")]
mod guest {
    wit_bindgen::generate!({
        path: "../../crates/noflex-plugin-wit/wit",
        world: "metadata-image-plugin",
        generate_all,
    });

    /// Embedded plugin manifest, preserved as a custom section in the
    /// final component so the host can read it without a post-build step.
    #[unsafe(link_section = "noflex:manifest")]
    #[used]
    static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
        *include_bytes!("manifest.toml");

    use std::{cell::RefCell, time::Duration};

    use exports::noflex::plugin::{background, image, metadata};
    use noflex::plugin::{
        host_net, host_secrets,
        types::{
            ExternalId, ExternalIdKind, ImageKind, MetadataImage, MetadataPerson, PluginError,
        },
    };
    use noflex_plugin_sdk::cache::TtlCache;

    use crate::api::{self, ArtKind, CandidateHandle, IdKind, SearchKind, TvdbId};

    const TIMEOUT_MS: u32 = 15_000;
    const IMAGE_TIMEOUT_MS: u32 = 60_000;

    thread_local! {
        /// Read once: every secret read is audit-logged host-side.
        static TOKEN: RefCell<Option<String>> = const { RefCell::new(None) };
        /// Ten minutes covers the host fetching each kind of one record.
        static ART: RefCell<TtlCache<String, Vec<api::ArtEntry>>> =
            RefCell::new(TtlCache::new(64, Duration::from_secs(600)));
        /// Every season and episode of a series needs its season ids.
        static SEASONS: RefCell<TtlCache<u64, Vec<api::SeasonRef>>> =
            RefCell::new(TtlCache::new(64, Duration::from_secs(600)));
        /// Person id → photo URL from the credits a fetch just parsed, so the
        /// host's photo requests that follow need no person request.
        static PHOTOS: RefCell<TtlCache<u64, String>> =
            RefCell::new(TtlCache::new(1024, Duration::from_secs(3600)));
    }

    fn remember_photos(payload: &api::Payload) {
        PHOTOS.with(|c| {
            let mut cache = c.borrow_mut();
            for person in &payload.people {
                if let (Some(id), Some(photo)) = (person.id, &person.photo) {
                    cache.put(id, photo.clone());
                }
            }
        });
    }

    /// The photo URL of TVDB person `id`.
    async fn photo_of(id: u64) -> Result<String, PluginError> {
        if let Some(photo) = PHOTOS.with(|c| c.borrow_mut().get(&id)) {
            return Ok(photo);
        }
        let body = get_api(api::person_url(id)).await?;
        let photo = api::parse_person_photo(&body)
            .map_err(PluginError::Internal)?
            .ok_or(PluginError::NotFound)?;
        PHOTOS.with(|c| c.borrow_mut().put(id, photo.clone()));
        Ok(photo)
    }

    /// Every image `tvdb` lists, from the cache or its record.
    async fn artwork(tvdb: TvdbId) -> Result<Vec<api::ArtEntry>, PluginError> {
        if let Some(entries) = ART.with(|c| c.borrow_mut().get(&tvdb.path())) {
            return Ok(entries);
        }
        let (_, entries) = details(tvdb).await?;
        ART.with(|c| c.borrow_mut().put(tvdb.path(), entries.clone()));
        Ok(entries)
    }

    /// `bytes` as the read end of a stream the host can drain.
    fn stream_of(bytes: Vec<u8>) -> wit_bindgen::StreamReader<u8> {
        let (mut writer, reader) = wit_stream::new::<u8>();
        // The host reads only after the export returns the read end, so an
        // inline (backpressured) write would deadlock.
        wit_bindgen::spawn_local(async move {
            writer.write_all(bytes).await;
        });
        reader
    }

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    async fn secret(key: &str) -> Option<String> {
        let value = host_secrets::read(key.into()).await.ok()?;
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    }

    /// A bearer token, logging in when none is held.
    async fn token() -> Result<String, PluginError> {
        if let Some(token) = TOKEN.with(|t| t.borrow().clone()) {
            return Ok(token);
        }
        let key = secret("api_key").await;
        let key = api::key_to_use(key.as_deref());
        let pin = secret("pin").await;
        let response = host_net::request(host_net::HttpRequest {
            method: "POST".into(),
            url: format!("{}/login", api::API_BASE),
            headers: vec![
                ("accept".into(), "application/json".into()),
                ("content-type".into(), "application/json".into()),
                ("user-agent".into(), api::USER_AGENT.into()),
            ],
            body: Some(api::login_body(key, pin.as_deref())),
            timeout_ms: Some(TIMEOUT_MS),
        })
        .await
        .map_err(|e| PluginError::Unavailable(format!("tvdb login failed: {e:?}")))?;
        match response.status {
            200 => {
                let token = api::parse_login(&response.body).map_err(PluginError::Internal)?;
                TOKEN.with(|t| *t.borrow_mut() = Some(token.clone()));
                Ok(token)
            }
            401 => Err(PluginError::Unavailable(
                "tvdb rejected the API key or PIN".into(),
            )),
            status => Err(PluginError::Unavailable(format!(
                "tvdb login answered {status}"
            ))),
        }
    }

    async fn get_api(url: String) -> Result<Vec<u8>, PluginError> {
        // A token expires after a month, or sooner if revoked: one fresh login.
        for fresh_login in [false, true] {
            let token = token().await?;
            let response = host_net::request(host_net::HttpRequest {
                method: "GET".into(),
                url: url.clone(),
                headers: vec![
                    ("accept".into(), "application/json".into()),
                    ("authorization".into(), format!("Bearer {token}")),
                    ("user-agent".into(), api::USER_AGENT.into()),
                ],
                body: None,
                timeout_ms: Some(TIMEOUT_MS),
            })
            .await
            .map_err(|e| PluginError::Unavailable(format!("tvdb request failed: {e:?}")))?;
            match response.status {
                200 => return Ok(response.body),
                401 if !fresh_login => TOKEN.with(|t| *t.borrow_mut() = None),
                401 => {
                    return Err(PluginError::Unavailable(
                        "tvdb refused a fresh token".into(),
                    ));
                }
                404 => return Err(PluginError::NotFound),
                429 => {
                    let retry_after = header(&response.headers, "retry-after")
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1);
                    return Err(PluginError::RateLimited(retry_after));
                }
                status => {
                    return Err(PluginError::Unavailable(format!("tvdb answered {status}")));
                }
            }
        }
        unreachable!("the loop's last pass returns")
    }

    /// Image CDN bytes plus the response content type.
    async fn download(url: String) -> Result<(Vec<u8>, Option<String>), PluginError> {
        let response = host_net::request(host_net::HttpRequest {
            method: "GET".into(),
            url,
            headers: vec![("user-agent".into(), api::USER_AGENT.into())],
            body: None,
            timeout_ms: Some(IMAGE_TIMEOUT_MS),
        })
        .await
        .map_err(|e| PluginError::Unavailable(format!("tvdb image request failed: {e:?}")))?;
        match response.status {
            200 => {
                let mime = header(&response.headers, "content-type").map(str::to_string);
                Ok((response.body, mime))
            }
            404 => Err(PluginError::NotFound),
            status => Err(PluginError::Unavailable(format!(
                "tvdb image CDN answered {status}"
            ))),
        }
    }

    /// The TVDB record behind `id`: TVDB ids parse directly, IMDb and TMDb
    /// ids resolve through remote-id search.
    async fn resolve(id: &ExternalId) -> Result<TvdbId, PluginError> {
        let remote = match id.kind {
            ExternalIdKind::Tvdb => return TvdbId::parse(&id.value).map_err(PluginError::BadInput),
            ExternalIdKind::Imdb => id.value.as_str(),
            ExternalIdKind::Tmdb => {
                api::tmdb_record_number(&id.value).ok_or(PluginError::NotFound)?
            }
            _ => {
                return Err(PluginError::BadInput(
                    "tvdb: unsupported external id kind".into(),
                ));
            }
        };
        let body = get_api(api::remote_id_url(remote)).await?;
        api::parse_remote_id(&body)
            .map_err(PluginError::Internal)?
            .ok_or(PluginError::NotFound)
    }

    /// The series' seasons in its default order, from the cache or `body`'s request.
    async fn seasons_of(series: u64) -> Result<Vec<api::SeasonRef>, PluginError> {
        if let Some(seasons) = SEASONS.with(|c| c.borrow_mut().get(&series)) {
            return Ok(seasons);
        }
        let body = get_api(api::series_url(series)).await?;
        let seasons = api::parse_seasons(&body).map_err(PluginError::Internal)?;
        SEASONS.with(|c| c.borrow_mut().put(series, seasons.clone()));
        Ok(seasons)
    }

    async fn season_id(series: u64, season: u32) -> Result<u64, PluginError> {
        seasons_of(series)
            .await?
            .iter()
            .find(|s| s.number == season)
            .map(|s| s.id)
            .ok_or(PluginError::NotFound)
    }

    async fn episode_id(series: u64, season: u32, episode: u32) -> Result<u64, PluginError> {
        let body = get_api(api::episode_lookup_url(series, season, episode)).await?;
        api::parse_episode_lookup(&body)
            .map_err(PluginError::Internal)?
            .ok_or(PluginError::NotFound)
    }

    /// The record's payload and every image it lists.
    async fn details(id: TvdbId) -> Result<(api::Payload, Vec<api::ArtEntry>), PluginError> {
        match id {
            TvdbId::Movie(movie) => {
                let body = get_api(api::movie_url(movie)).await?;
                api::parse_movie(&body).map_err(PluginError::Internal)
            }
            TvdbId::Series(series) => {
                let body = get_api(api::series_url(series)).await?;
                if let Ok(seasons) = api::parse_seasons(&body) {
                    SEASONS.with(|c| c.borrow_mut().put(series, seasons));
                }
                api::parse_series(&body).map_err(PluginError::Internal)
            }
            TvdbId::Season { series, season } => {
                let record = season_id(series, season).await?;
                let body = get_api(api::season_url(record)).await?;
                api::parse_season(series, season, &body).map_err(PluginError::Internal)
            }
            TvdbId::Episode {
                series,
                season,
                episode,
            } => {
                let record = episode_id(series, season, episode).await?;
                let body = get_api(api::episode_url(record)).await?;
                api::parse_episode(series, season, episode, &body).map_err(PluginError::Internal)
            }
        }
    }

    fn id_kind(kind: IdKind) -> ExternalIdKind {
        match kind {
            IdKind::Tvdb => ExternalIdKind::Tvdb,
            IdKind::Imdb => ExternalIdKind::Imdb,
            IdKind::Tmdb => ExternalIdKind::Tmdb,
        }
    }

    fn wit_kind(kind: ArtKind) -> ImageKind {
        match kind {
            ArtKind::Primary => ImageKind::Primary,
            ArtKind::Backdrop => ImageKind::Backdrop,
            ArtKind::Logo => ImageKind::Logo,
            ArtKind::Thumb => ImageKind::Thumb,
        }
    }

    fn art_kind(kind: ImageKind) -> Option<ArtKind> {
        Some(match kind {
            ImageKind::Primary => ArtKind::Primary,
            ImageKind::Backdrop => ArtKind::Backdrop,
            ImageKind::Logo => ArtKind::Logo,
            ImageKind::Thumb => ArtKind::Thumb,
            _ => return None,
        })
    }

    fn to_wit(payload: api::Payload) -> metadata::MetadataPayload {
        metadata::MetadataPayload {
            external_ids: payload
                .ids
                .into_iter()
                .map(|(kind, value)| ExternalId {
                    kind: id_kind(kind),
                    value,
                })
                .collect(),
            title: payload.title,
            sort_title: None,
            original_title: payload.original_title,
            year: payload.year,
            release_date: payload.release_date,
            overview: payload.overview,
            tagline: payload.tagline,
            runtime_ms: payload.runtime_ms,
            genres: payload.genres,
            studios: payload.studios,
            people: payload
                .people
                .into_iter()
                .map(|p| MetadataPerson {
                    name: p.name,
                    role: p.role,
                    character: p.character,
                    external_ids: p
                        .id
                        .map(|id| ExternalId {
                            kind: ExternalIdKind::Tvdb,
                            value: format!("people/{id}"),
                        })
                        .into_iter()
                        .collect(),
                })
                .collect(),
            images: payload
                .images
                .into_iter()
                .map(|a| MetadataImage {
                    kind: wit_kind(a.kind),
                    url: a.url,
                    language: a.language,
                    width: a.width,
                    height: a.height,
                })
                .collect(),
            rating: payload.rating,
            ratings: Vec::new(),
            original_language: payload.original_language,
            origin_countries: payload.origin_countries,
            keywords: Vec::new(),
            budget_usd: payload.budget_usd,
        }
    }

    struct Tvdb;

    impl metadata::Guest for Tvdb {
        fn attribution() -> Option<metadata::ProviderAttribution> {
            Some(metadata::ProviderAttribution {
                // TheTVDB's sample wording. Its terms ask for a direct link
                // shown to whoever is viewing the metadata.
                text: "Metadata provided by TheTVDB. Please consider adding missing information or subscribing."
                    .into(),
                url: Some("https://thetvdb.com/subscribe".into()),
                logo: Some(include_bytes!("../assets/tvdb-logo.png").to_vec()),
            })
        }

        fn supports(kind: metadata::WorkKind) -> bool {
            matches!(
                kind,
                metadata::WorkKind::Movie
                    | metadata::WorkKind::Series
                    | metadata::WorkKind::Season
                    | metadata::WorkKind::Episode,
            )
        }

        async fn search(
            query: metadata::SearchQuery,
        ) -> Result<Vec<metadata::MetadataMatch>, metadata::PluginError> {
            let kind = match query.kind {
                metadata::WorkKind::Movie => SearchKind::Movie,
                metadata::WorkKind::Series => SearchKind::Series,
                metadata::WorkKind::Season | metadata::WorkKind::Episode => {
                    let episode = query.kind == metadata::WorkKind::Episode;
                    return Ok(api::child_from_hints(&query.hints, episode)
                        .map(|id| metadata::MetadataMatch {
                            external_id: ExternalId {
                                kind: ExternalIdKind::Tvdb,
                                value: id.path(),
                            },
                            title: query.title.clone(),
                            year: query.year,
                            confidence: 1.0,
                        })
                        .into_iter()
                        .collect());
                }
                _ => {
                    return Err(PluginError::BadInput("tvdb: unsupported work kind".into()));
                }
            };
            let body = get_api(api::search_url(kind, &query.title, query.year)).await?;
            let matches = api::parse_search(kind, &body, &query.title, query.year)
                .map_err(PluginError::Internal)?;
            Ok(matches
                .into_iter()
                .map(|m| metadata::MetadataMatch {
                    external_id: ExternalId {
                        kind: ExternalIdKind::Tvdb,
                        value: m.id.path(),
                    },
                    title: m.title,
                    year: m.year,
                    confidence: m.confidence,
                })
                .collect())
        }

        async fn fetch(
            id: metadata::ExternalId,
        ) -> Result<metadata::MetadataPayload, metadata::PluginError> {
            let tvdb = resolve(&id).await?;
            let (payload, art) = details(tvdb).await?;
            ART.with(|c| c.borrow_mut().put(tvdb.path(), art));
            remember_photos(&payload);
            Ok(to_wit(payload))
        }

        async fn fetch_children(
            parent: metadata::ExternalId,
            _parent_work: metadata::WorkId,
        ) -> Result<Vec<metadata::MetadataPayload>, metadata::PluginError> {
            let children = match resolve(&parent).await? {
                TvdbId::Series(series) => {
                    let body = get_api(api::series_url(series)).await?;
                    api::parse_series_children(series, &body).map_err(PluginError::Internal)?
                }
                TvdbId::Season { series, season } => {
                    let record = season_id(series, season).await?;
                    let body = get_api(api::season_url(record)).await?;
                    api::parse_season_children(series, season, &body)
                        .map_err(PluginError::Internal)?
                }
                TvdbId::Movie(_) | TvdbId::Episode { .. } => Vec::new(),
            };
            children.iter().for_each(remember_photos);
            Ok(children.into_iter().map(to_wit).collect())
        }
    }

    impl image::Guest for Tvdb {
        async fn fetch(
            id: image::ExternalId,
            kind: image::ImageKind,
            language: Option<String>,
        ) -> Result<(image::ImageMeta, wit_bindgen::StreamReader<u8>), image::PluginError> {
            if id.kind == ExternalIdKind::Tvdb
                && let Some(person) = api::person_id(&id.value)
            {
                if kind != ImageKind::Primary {
                    return Err(PluginError::NotFound);
                }
                let url = photo_of(person).await?;
                let (bytes, content_type) = download(url.clone()).await?;
                let meta = image::ImageMeta {
                    kind,
                    language: None,
                    mime: api::image_mime(content_type.as_deref(), &url),
                    width: None,
                    height: None,
                };
                return Ok((meta, stream_of(bytes)));
            }
            let art = art_kind(kind).ok_or(PluginError::NotFound)?;
            let tvdb = resolve(&id).await?;
            let entries = artwork(tvdb).await?;
            let chosen = api::choose(&entries, tvdb, art, language.as_deref())
                .ok_or(PluginError::NotFound)?;
            let (bytes, content_type) = download(chosen.url.clone()).await?;
            let meta = image::ImageMeta {
                kind,
                language: chosen.language,
                mime: api::image_mime(content_type.as_deref(), &chosen.url),
                width: chosen.width,
                height: chosen.height,
            };
            Ok((meta, stream_of(bytes)))
        }

        async fn list_candidates(
            id: image::ExternalId,
            kind: image::ImageKind,
        ) -> Result<Vec<image::ImageCandidate>, image::PluginError> {
            let Some(art) = art_kind(kind) else {
                return Ok(Vec::new());
            };
            let tvdb = resolve(&id).await?;
            let entries = artwork(tvdb).await?;
            Ok(api::candidates(&entries, tvdb, art)
                .into_iter()
                .map(|c| image::ImageCandidate {
                    handle: c.handle,
                    kind,
                    language: c.language,
                    width: c.width,
                    height: c.height,
                    score: Some(c.score),
                })
                .collect())
        }

        async fn fetch_candidate(
            handle: String,
            preview: bool,
        ) -> Result<(image::ImageMeta, wit_bindgen::StreamReader<u8>), image::PluginError> {
            let handle = CandidateHandle::decode(&handle).map_err(PluginError::BadInput)?;
            let url = handle.url(preview).to_string();
            let (bytes, content_type) = download(url.clone()).await?;
            let (width, height) = if url == handle.image {
                (handle.width, handle.height)
            } else {
                (None, None)
            };
            let meta = image::ImageMeta {
                kind: wit_kind(handle.kind),
                language: handle.language,
                mime: api::image_mime(content_type.as_deref(), &url),
                width,
                height,
            };
            Ok((meta, stream_of(bytes)))
        }
    }

    // -------- Background hook (stub — this plugin registers no [[tasks]]) --------

    impl background::Guest for Tvdb {
        async fn run_task(
            task: String,
            cursor: Option<String>,
            payload: Option<String>,
        ) -> Result<background::TaskOutcome, background::PluginError> {
            let _ = (cursor, payload);
            Err(background::PluginError::BadInput(format!(
                "tvdb: unknown background task `{task}`"
            )))
        }
    }

    export!(Tvdb);
}
