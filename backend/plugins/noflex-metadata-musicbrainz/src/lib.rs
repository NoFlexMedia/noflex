// SPDX-License-Identifier: AGPL-3.0-only

mod api;
mod pace;

#[cfg(target_arch = "wasm32")]
mod guest {
    // The canonical `metadata-plugin` world plus WASI 0.3's monotonic clock,
    // whose async `wait-until` lets the pacer sleep without blocking the
    // instance's other calls.
    wit_bindgen::generate!({
        inline: r#"
            package noflex:musicbrainz;

            world musicbrainz {
                include noflex:plugin/metadata-plugin@0.8.0;
                import wasi:clocks/monotonic-clock@0.3.0;
            }
        "#,
        path: "../../crates/noflex-plugin-wit/wit",
        world: "noflex:musicbrainz/musicbrainz",
        generate_all,
    });

    #[unsafe(link_section = "noflex:manifest")]
    #[used]
    static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
        *include_bytes!("manifest.toml");

    use exports::noflex::plugin::{background, metadata};
    use noflex::plugin::{
        host_net,
        types::{ExternalIdKind, MetadataPerson},
    };

    use crate::{
        api::{self, EntityKind, IdKind},
        pace::Pacer,
    };

    const TIMEOUT_MS: u32 = 15_000;

    thread_local! {
        /// MusicBrainz allows 1 request/s; queue at most 10s of requests so
        /// a waiting call stays well inside the host's call deadline.
        static PACER: Pacer = const { Pacer::new(1_000_000_000, 10_000_000_000) };
    }

    /// Wait for this request's send slot, or answer `rate-limited` when the
    /// queue is already too long.
    async fn paced() -> Result<(), metadata::PluginError> {
        let now = wasi::clocks::monotonic_clock::now();
        let slot = PACER
            .with(|pacer| pacer.reserve(now))
            .map_err(metadata::PluginError::RateLimited)?;
        if slot > now {
            wasi::clocks::monotonic_clock::wait_until(slot).await;
        }
        Ok(())
    }

    struct MusicBrainz;

    fn entity_kind(kind: metadata::WorkKind) -> Option<EntityKind> {
        match kind {
            metadata::WorkKind::Artist => Some(EntityKind::Artist),
            metadata::WorkKind::Album => Some(EntityKind::Album),
            metadata::WorkKind::Track => Some(EntityKind::Track),
            _ => None,
        }
    }

    fn external_id_kind(kind: IdKind) -> ExternalIdKind {
        match kind {
            IdKind::Artist => ExternalIdKind::MusicBrainzArtist,
            IdKind::ReleaseGroup => ExternalIdKind::MusicBrainzReleaseGroup,
            IdKind::Recording => ExternalIdKind::MusicBrainzRecording,
        }
    }

    fn kind_from_external_id(
        id: &metadata::ExternalId,
    ) -> Result<EntityKind, metadata::PluginError> {
        match id.kind {
            ExternalIdKind::MusicBrainzArtist => Ok(EntityKind::Artist),
            ExternalIdKind::MusicBrainzReleaseGroup => Ok(EntityKind::Album),
            ExternalIdKind::MusicBrainzRecording => Ok(EntityKind::Track),
            _ => Err(metadata::PluginError::BadInput(
                "musicbrainz: unsupported external id kind".into(),
            )),
        }
    }

    fn request(url: String) -> host_net::HttpRequest {
        host_net::HttpRequest {
            method: "GET".into(),
            url,
            headers: api::headers(),
            body: None,
            timeout_ms: Some(TIMEOUT_MS),
        }
    }

    async fn get(url: String) -> Result<Vec<u8>, metadata::PluginError> {
        paced().await?;
        let response = host_net::request(request(url)).await.map_err(|error| {
            metadata::PluginError::Unavailable(format!("musicbrainz request failed: {error:?}"))
        })?;
        match response.status {
            200 => Ok(response.body),
            404 => Err(metadata::PluginError::NotFound),
            // MusicBrainz signals its rate limit with 503 (429 is accepted
            // too, should it ever switch).
            429 | 503 => {
                let retry_after = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
                    .and_then(|(_, value)| value.parse().ok())
                    .unwrap_or(1);
                PACER.with(|pacer| {
                    pacer.back_off(wasi::clocks::monotonic_clock::now(), retry_after)
                });
                Err(metadata::PluginError::RateLimited(retry_after))
            }
            status => Err(metadata::PluginError::Unavailable(format!(
                "musicbrainz answered {status}"
            ))),
        }
    }

    impl metadata::Guest for MusicBrainz {
        fn supports(kind: metadata::WorkKind) -> bool {
            entity_kind(kind).is_some()
        }

        async fn search(
            query: metadata::SearchQuery,
        ) -> Result<Vec<metadata::MetadataMatch>, metadata::PluginError> {
            let kind = entity_kind(query.kind).ok_or_else(|| {
                metadata::PluginError::BadInput("musicbrainz: unsupported work kind".into())
            })?;
            let body = get(api::search_url(kind, &query.title, query.year)).await?;
            api::parse_search(kind, &body)
                .map(|matches| {
                    matches
                        .into_iter()
                        .map(|item| metadata::MetadataMatch {
                            external_id: metadata::ExternalId {
                                kind: external_id_kind(item.id_kind),
                                value: item.id,
                            },
                            title: item.title,
                            year: item.year,
                            confidence: item.confidence,
                        })
                        .collect()
                })
                .map_err(metadata::PluginError::Internal)
        }

        async fn fetch(
            id: metadata::ExternalId,
        ) -> Result<metadata::MetadataPayload, metadata::PluginError> {
            let kind = kind_from_external_id(&id)?;
            let url = api::fetch_url(kind, &id.value).map_err(metadata::PluginError::BadInput)?;
            let body = get(url).await?;
            let payload = api::parse_fetch(kind, &body).map_err(metadata::PluginError::Internal)?;
            Ok(metadata::MetadataPayload {
                external_ids: vec![metadata::ExternalId {
                    kind: external_id_kind(payload.external_id_kind),
                    value: payload.external_id,
                }],
                title: payload.title,
                sort_title: payload.sort_title,
                original_title: None,
                year: payload.year,
                release_date: payload.release_date,
                overview: payload.overview,
                tagline: None,
                runtime_ms: payload.runtime_ms,
                genres: payload.genres,
                studios: Vec::new(),
                people: payload
                    .people
                    .into_iter()
                    .map(|person| MetadataPerson {
                        name: person.name,
                        role: person.role,
                        character: None,
                        photo_url: None,
                    })
                    .collect(),
                images: Vec::new(),
                rating: None,
                community_rating: None,
            })
        }

        async fn fetch_children(
            _parent: metadata::ExternalId,
            _parent_work: metadata::WorkId,
        ) -> Result<Vec<metadata::MetadataPayload>, metadata::PluginError> {
            Ok(Vec::new())
        }
    }

    impl background::Guest for MusicBrainz {
        async fn run_task(
            task: String,
            cursor: Option<String>,
            payload: Option<String>,
        ) -> Result<background::TaskOutcome, background::PluginError> {
            let _ = (cursor, payload);
            Err(background::PluginError::BadInput(format!(
                "musicbrainz: unknown background task `{task}`"
            )))
        }
    }

    export!(MusicBrainz);
}
