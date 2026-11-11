// SPDX-License-Identifier: AGPL-3.0-only
//
// TMDb metadata + image provider — first-party WASM plugin.
//
// All egress goes through `host-net` to the manifest's two hosts: the v3
// API for search/details/image listings and the image CDN for bytes. The
// API key is NoFlex's shared one unless an operator sets the `api_key`
// secret (a v4 read token or a v3 key), i.e. `tmdb.toml` in a secret tier.
// Request building and parsing live in `api` so they are testable
// off-target; this module is only the WIT glue. External-id conventions
// are documented at the top of `api.rs`.
//
// Season and episode matches need no request: the host's search hints
// name the parent series' TMDb id and the season/episode numbers, from
// which the child's id follows; `fetch` then fails for numbers TMDb does
// not have.
//
// Credited people carry `person/{id}` ids, which `image::fetch` answers
// with their photo: from the credit's own profile path when a recent
// fetch saw one, else from the person's photo listing.

mod api;
mod pace;

#[cfg(target_arch = "wasm32")]
mod guest {
    // The canonical `metadata-image-plugin` world plus WASI 0.3's
    // monotonic clock, whose async `wait-until` lets the pacer sleep
    // without blocking the instance's other calls.
    wit_bindgen::generate!({
        inline: r#"
            package noflex:tmdb;

            world tmdb {
                include noflex:plugin/metadata-image-plugin@0.2.0;
                import wasi:clocks/monotonic-clock@0.3.0;
            }
        "#,
        path: "../../crates/noflex-plugin-wit/wit",
        world: "noflex:tmdb/tmdb",
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
            Rating,
        },
    };
    use noflex_plugin_sdk::cache::TtlCache;

    use crate::{
        api::{self, ArtKind, CandidateHandle, FindSource, IdKind, Images, SearchKind, TmdbId},
        pace::Pacer,
    };

    const TIMEOUT_MS: u32 = 15_000;
    const IMAGE_TIMEOUT_MS: u32 = 60_000;
    const SECRET_KEY: &str = "api_key";

    thread_local! {
        /// ~40 requests/s against TMDb's ~50; queue at most 10s so a
        /// waiting call stays inside the host's call deadline.
        static PACER: Pacer = const { Pacer::new(25_000_000, 10_000_000_000) };
        /// Read once: every secret read is audit-logged host-side.
        static API_KEY: RefCell<Option<String>> = const { RefCell::new(None) };
        /// Ten minutes covers the host fetching each kind of one record.
        static LISTINGS: RefCell<TtlCache<String, Images>> =
            RefCell::new(TtlCache::new(64, Duration::from_secs(600)));
        /// Person id → photo path from the credits a fetch just parsed, so
        /// the host's photo requests that follow need no listing request.
        static PROFILES: RefCell<TtlCache<u64, String>> =
            RefCell::new(TtlCache::new(1024, Duration::from_secs(3600)));
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

    async fn api_key() -> Result<String, PluginError> {
        if let Some(key) = API_KEY.with(|k| k.borrow().clone()) {
            return Ok(key);
        }
        let secret = match host_secrets::read(SECRET_KEY.into()).await {
            Ok(value) => Some(value),
            // The host's answer when no tier sets it, which is the usual case.
            Err(PluginError::Unavailable(_)) => None,
            Err(e) => {
                return Err(PluginError::Unavailable(format!(
                    "tmdb: secret `{SECRET_KEY}` unreadable: {e:?}"
                )));
            }
        };
        let key = api::key_to_use(secret.as_deref()).to_string();
        API_KEY.with(|k| *k.borrow_mut() = Some(key.clone()));
        Ok(key)
    }

    fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    async fn get_api(url: String) -> Result<Vec<u8>, PluginError> {
        let key = api_key().await?;
        paced().await?;
        let (url, headers) = api::authorize(&url, &key);
        let response = host_net::request(host_net::HttpRequest {
            method: "GET".into(),
            url,
            headers,
            body: None,
            timeout_ms: Some(TIMEOUT_MS),
        })
        .await
        .map_err(|e| PluginError::Unavailable(format!("tmdb request failed: {e:?}")))?;
        match response.status {
            200 => Ok(response.body),
            // A rotated key needs a fresh read.
            401 => {
                API_KEY.with(|k| *k.borrow_mut() = None);
                Err(PluginError::Unavailable("tmdb rejected the API key".into()))
            }
            404 => Err(PluginError::NotFound),
            429 => {
                let retry_after = header(&response.headers, "retry-after")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(1);
                PACER.with(|pacer| pacer.back_off(now(), retry_after));
                Err(PluginError::RateLimited(retry_after))
            }
            status => Err(PluginError::Unavailable(format!("tmdb answered {status}"))),
        }
    }

    /// Image CDN bytes plus the response content type. The CDN needs no
    /// key and is not rate-limited like the API.
    async fn download(url: String) -> Result<(Vec<u8>, Option<String>), PluginError> {
        let response = host_net::request(host_net::HttpRequest {
            method: "GET".into(),
            url,
            headers: vec![("user-agent".into(), api::USER_AGENT.into())],
            body: None,
            timeout_ms: Some(IMAGE_TIMEOUT_MS),
        })
        .await
        .map_err(|e| PluginError::Unavailable(format!("tmdb image request failed: {e:?}")))?;
        match response.status {
            200 => {
                let mime = header(&response.headers, "content-type").map(str::to_string);
                Ok((response.body, mime))
            }
            404 => Err(PluginError::NotFound),
            status => Err(PluginError::Unavailable(format!(
                "tmdb image CDN answered {status}"
            ))),
        }
    }

    /// The TMDb record behind `id`: TMDb ids parse directly, IMDb and TVDB
    /// ids resolve through `/find`.
    async fn resolve(id: &ExternalId) -> Result<TmdbId, PluginError> {
        let (source, value) = match id.kind {
            ExternalIdKind::Tmdb => {
                return TmdbId::parse(&id.value).map_err(PluginError::BadInput);
            }
            ExternalIdKind::Imdb => (FindSource::Imdb, id.value.as_str()),
            ExternalIdKind::Tvdb => (
                FindSource::Tvdb,
                api::tvdb_record_number(&id.value).ok_or(PluginError::NotFound)?,
            ),
            _ => {
                return Err(PluginError::BadInput(
                    "tmdb: unsupported external id kind".into(),
                ));
            }
        };
        let body = get_api(api::find_url(source, value)).await?;
        api::parse_find(&body, source == FindSource::Tvdb)
            .map_err(PluginError::Internal)?
            .ok_or(PluginError::NotFound)
    }

    fn id_kind(kind: IdKind) -> ExternalIdKind {
        match kind {
            IdKind::Tmdb => ExternalIdKind::Tmdb,
            IdKind::Imdb => ExternalIdKind::Imdb,
            IdKind::Tvdb => ExternalIdKind::Tvdb,
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

    fn remember_profiles(payload: &api::Payload) {
        PROFILES.with(|c| {
            let mut cache = c.borrow_mut();
            for person in &payload.people {
                if let (Some(id), Some(path)) = (person.id, &person.profile) {
                    cache.put(id, path.clone());
                }
            }
        });
    }

    /// The photo of TMDb person `id` as a CDN file path.
    async fn profile_path(id: u64) -> Result<String, PluginError> {
        if let Some(path) = PROFILES.with(|c| c.borrow_mut().get(&id)) {
            return Ok(path);
        }
        let body = get_api(api::person_images_url(id)).await?;
        let images = api::parse_images(&body).map_err(PluginError::Internal)?;
        let path = api::best_profile(&images).ok_or(PluginError::NotFound)?;
        PROFILES.with(|c| c.borrow_mut().put(id, path.clone()));
        Ok(path)
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

    /// The image listing of `tmdb` narrowed to `language` (all languages
    /// when `None`), from the cache or a request.
    async fn listing(tmdb: TmdbId, language: Option<&str>) -> Result<Images, PluginError> {
        let key = listing_key(tmdb, language);
        if let Some(images) = LISTINGS.with(|c| c.borrow_mut().get(&key)) {
            return Ok(images);
        }
        let body = get_api(api::images_url(tmdb, language)).await?;
        let images = api::parse_images(&body).map_err(PluginError::Internal)?;
        LISTINGS.with(|c| c.borrow_mut().put(key, images.clone()));
        Ok(images)
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
                            kind: ExternalIdKind::Tmdb,
                            value: format!("person/{id}"),
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
            ratings: payload
                .score
                .map(|score| Rating {
                    source: "tmdb".into(),
                    value: score.average,
                    scale: 10.0,
                    votes: score.votes,
                })
                .into_iter()
                .collect(),
            original_language: payload.original_language,
            origin_countries: payload.origin_countries,
            keywords: payload.keywords,
            budget_usd: payload.budget_usd,
        }
    }

    fn listing_key(id: TmdbId, language: Option<&str>) -> String {
        format!("{}|{}", id.path(), language.unwrap_or("*"))
    }

    struct Tmdb;

    impl metadata::Guest for Tmdb {
        fn attribution() -> Option<metadata::ProviderAttribution> {
            Some(metadata::ProviderAttribution {
                // TMDB's required clause, word for word, with "product" for its
                // bracket. Their terms also ask that the logo is not recoloured
                // or reshaped.
                text: "This product uses TMDB and the TMDB APIs but is not endorsed, certified, or otherwise approved by TMDB."
                    .into(),
                url: Some("https://www.themoviedb.org".into()),
                logo: Some(include_bytes!("../assets/tmdb-logo.svg").to_vec()),
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
                                kind: ExternalIdKind::Tmdb,
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
                    return Err(PluginError::BadInput("tmdb: unsupported work kind".into()));
                }
            };
            let body = get_api(api::search_url(kind, &query.title, query.year)).await?;
            let matches = api::parse_search(kind, &body, &query.title, query.year)
                .map_err(PluginError::Internal)?;
            Ok(matches
                .into_iter()
                .map(|m| metadata::MetadataMatch {
                    external_id: ExternalId {
                        kind: ExternalIdKind::Tmdb,
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
            let tmdb = resolve(&id).await?;
            let body = get_api(api::details_url(tmdb)).await?;
            let payload = api::parse_details(tmdb, &body).map_err(PluginError::Internal)?;
            // The details response already carries the English listing.
            if let Ok(images) = api::parse_images_appended(&body) {
                LISTINGS.with(|c| c.borrow_mut().put(listing_key(tmdb, Some("en")), images));
            }
            remember_profiles(&payload);
            Ok(to_wit(payload))
        }

        async fn fetch_children(
            parent: metadata::ExternalId,
            _parent_work: metadata::WorkId,
        ) -> Result<Vec<metadata::MetadataPayload>, metadata::PluginError> {
            let tmdb = resolve(&parent).await?;
            if !matches!(tmdb, TmdbId::Series(_) | TmdbId::Season { .. }) {
                return Ok(Vec::new());
            }
            let body = get_api(api::details_url(tmdb)).await?;
            let children = api::parse_children(tmdb, &body).map_err(PluginError::Internal)?;
            children.iter().for_each(remember_profiles);
            Ok(children.into_iter().map(to_wit).collect())
        }
    }

    impl image::Guest for Tmdb {
        async fn fetch(
            id: image::ExternalId,
            kind: image::ImageKind,
            language: Option<String>,
        ) -> Result<(image::ImageMeta, wit_bindgen::StreamReader<u8>), image::PluginError> {
            if id.kind == ExternalIdKind::Tmdb
                && let Some(person) = api::person_id(&id.value)
            {
                if kind != ImageKind::Primary {
                    return Err(PluginError::NotFound);
                }
                let path = profile_path(person).await?;
                let url = api::image_url(api::PROFILE_SIZE, &path);
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
            let tmdb = resolve(&id).await?;
            if !api::kinds_for(tmdb).contains(&art) {
                return Err(PluginError::NotFound);
            }
            let lang = language.as_deref().and_then(api::primary_language);
            let images = listing(tmdb, lang.as_deref()).await?;
            let chosen =
                api::choose(&images, tmdb, art, lang.as_deref()).ok_or(PluginError::NotFound)?;
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
            let tmdb = resolve(&id).await?;
            if !api::kinds_for(tmdb).contains(&art) {
                return Ok(Vec::new());
            }
            let images = listing(tmdb, None).await?;
            Ok(api::candidates(&images, tmdb, art)
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
            let url = handle.url(preview);
            let (bytes, content_type) = download(url.clone()).await?;
            // A preview's size is the CDN's to choose; only the original's is known.
            let (width, height) = if preview {
                (None, None)
            } else {
                (handle.width, handle.height)
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

    impl background::Guest for Tmdb {
        async fn run_task(
            task: String,
            cursor: Option<String>,
            payload: Option<String>,
        ) -> Result<background::TaskOutcome, background::PluginError> {
            let _ = (cursor, payload);
            Err(background::PluginError::BadInput(format!(
                "tmdb: unknown background task `{task}`"
            )))
        }
    }

    export!(Tmdb);
}
