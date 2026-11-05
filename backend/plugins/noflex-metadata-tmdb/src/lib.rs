// SPDX-License-Identifier: AGPL-3.0-only
//
// TMDb metadata + image provider — first-party WASM plugin.
//
// All egress goes through `host-net` to the manifest's two hosts: the v3
// API for search/details/image listings and the image CDN for bytes. The
// API key (a v4 read token or a v3 key) is the `api_key` secret, i.e.
// `tmdb.toml` in a secret tier. Request building and parsing live in `api` so they are testable
// off-target; this module is only the WIT glue. External-id conventions
// are documented at the top of `api.rs`.
//
// Season and episode matches need no request: the host's search hints
// name the parent series' TMDb id and the season/episode numbers, from
// which the child's id follows; `fetch` then fails for numbers TMDb does
// not have.

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
                include noflex:plugin/metadata-image-plugin@0.1.0;
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
        },
    };
    use noflex_plugin_sdk::cache::TtlCache;

    use crate::{
        api::{self, ArtKind, FindSource, IdKind, Images, SearchKind, TmdbId},
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
        let key = host_secrets::read(SECRET_KEY.into())
            .await
            .map_err(|e| {
                PluginError::Unavailable(format!("tmdb: secret `{SECRET_KEY}` unreadable: {e:?}"))
            })?
            .trim()
            .to_string();
        if key.is_empty() {
            return Err(PluginError::Unavailable(format!(
                "tmdb: secret `{SECRET_KEY}` is empty"
            )));
        }
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
        let source = match id.kind {
            ExternalIdKind::Tmdb => {
                return TmdbId::parse(&id.value).map_err(PluginError::BadInput);
            }
            ExternalIdKind::Imdb => FindSource::Imdb,
            ExternalIdKind::Tvdb => FindSource::Tvdb,
            _ => {
                return Err(PluginError::BadInput(
                    "tmdb: unsupported external id kind".into(),
                ));
            }
        };
        let body = get_api(api::find_url(source, &id.value)).await?;
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
                    photo_url: p.photo_url,
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
            community_rating: payload.community_rating,
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
            Ok(children.into_iter().map(to_wit).collect())
        }
    }

    impl image::Guest for Tmdb {
        async fn fetch(
            id: image::ExternalId,
            kind: image::ImageKind,
            language: Option<String>,
        ) -> Result<(image::ImageMeta, wit_bindgen::StreamReader<u8>), image::PluginError> {
            let art = art_kind(kind).ok_or(PluginError::NotFound)?;
            let tmdb = resolve(&id).await?;
            if !api::kinds_for(tmdb).contains(&art) {
                return Err(PluginError::NotFound);
            }
            let lang = language.as_deref().and_then(api::primary_language);
            let key = listing_key(tmdb, lang.as_deref());
            let cached = LISTINGS.with(|c| c.borrow_mut().get(&key));
            let images = match cached {
                Some(images) => images,
                None => {
                    let body = get_api(api::images_url(tmdb, lang.as_deref())).await?;
                    let images = api::parse_images(&body).map_err(PluginError::Internal)?;
                    LISTINGS.with(|c| c.borrow_mut().put(key, images.clone()));
                    images
                }
            };
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
            let (mut writer, reader) = wit_stream::new::<u8>();
            // The host reads only after this returns the read end, so an
            // inline (backpressured) write would deadlock.
            wit_bindgen::spawn_local(async move {
                writer.write_all(bytes).await;
            });
            Ok((meta, reader))
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
