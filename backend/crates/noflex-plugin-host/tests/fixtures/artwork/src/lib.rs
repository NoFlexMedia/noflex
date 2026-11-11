// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for the host's metadata service: deterministic
// search confidences (a clear match, a near-tie, a weak hit), hint-
// derived season/episode ids, payloads listing art kinds, and real PNG
// bytes from `image::fetch`. Values containing `fail` make `fetch` fail;
// values containing `noart` have no images at all. A `language` of `xx`
// is answered with English art so the host's language fallback shows.
// Values containing `cast` credit two people with `person/<n>` ids, the
// second of whom (`person/2-noart`) has no photo. `list-candidates`
// offers two candidates per served kind, with handles
// `<value>|<kind>|<n>`; `fetch-candidate` answers any handle but
// `missing`, reporting a width of 1 for a preview and 2 otherwise.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "metadata-image-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{background, image, metadata};
use noflex::plugin::types::{
    ExternalId, ExternalIdKind, ImageKind, MetadataImage, MetadataPerson, PluginError,
};

/// A valid 1x1 RGBA PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

struct Fixture;

fn tmdb(value: String, title: String, confidence: f32) -> metadata::MetadataMatch {
    metadata::MetadataMatch {
        external_id: ExternalId {
            kind: ExternalIdKind::Tmdb,
            value,
        },
        title,
        year: None,
        confidence,
    }
}

fn hint<'a>(hints: &'a [String], prefix: &str) -> Option<&'a str> {
    hints.iter().find_map(|h| h.strip_prefix(prefix))
}

impl metadata::Guest for Fixture {
    fn attribution() -> Option<metadata::ProviderAttribution> {
        Some(metadata::ProviderAttribution {
            text: "Data from the fixture.".into(),
            url: Some("https://fixture.example".into()),
            logo: Some(PNG.to_vec()),
        })
    }

    fn supports(kind: metadata::WorkKind) -> bool {
        !matches!(kind, metadata::WorkKind::Track | metadata::WorkKind::Photo)
    }

    async fn search(
        query: metadata::SearchQuery,
    ) -> Result<Vec<metadata::MetadataMatch>, PluginError> {
        if matches!(
            query.kind,
            metadata::WorkKind::Season | metadata::WorkKind::Episode
        ) {
            let (Some(parent), Some(season)) = (
                hint(&query.hints, "parent-external-id:tmdb:"),
                hint(&query.hints, "season:"),
            ) else {
                return Ok(Vec::new());
            };
            let mut value = format!("{parent}/season/{season}");
            if query.kind == metadata::WorkKind::Episode {
                let Some(episode) = hint(&query.hints, "episode:") else {
                    return Ok(Vec::new());
                };
                value.push_str(&format!("/episode/{episode}"));
            }
            return Ok(vec![tmdb(value, query.title, 1.0)]);
        }
        Ok(match query.title.as_str() {
            "Ambiguous" => vec![
                tmdb("movie/1".into(), "Ambiguous".into(), 0.95),
                tmdb("movie/2".into(), "Ambiguous".into(), 0.94),
            ],
            "Weak" => vec![tmdb("movie/3".into(), "Weak".into(), 0.5)],
            title => {
                let prefix = if query.kind == metadata::WorkKind::Series {
                    "tv"
                } else {
                    "movie"
                };
                let slug = title.to_lowercase().replace(' ', "-");
                vec![tmdb(format!("{prefix}/{slug}"), title.into(), 0.99)]
            }
        })
    }

    async fn fetch(id: ExternalId) -> Result<metadata::MetadataPayload, PluginError> {
        if id.value.contains("fail") {
            return Err(PluginError::Unavailable("fixture: fetch fails".into()));
        }
        let images = if id.value.contains("noart") {
            Vec::new()
        } else {
            vec![
                MetadataImage {
                    kind: ImageKind::Primary,
                    url: "https://example.invalid/poster.png".into(),
                    language: Some("en".into()),
                    width: Some(1),
                    height: Some(1),
                },
                MetadataImage {
                    kind: ImageKind::Backdrop,
                    url: "https://example.invalid/backdrop.png".into(),
                    language: None,
                    width: Some(1),
                    height: Some(1),
                },
            ]
        };
        let people = if id.value.contains("cast") {
            ["1", "2-noart"]
                .into_iter()
                .map(|n| MetadataPerson {
                    name: format!("Person {n}"),
                    role: Some("Actor".into()),
                    character: None,
                    external_ids: vec![ExternalId {
                        kind: ExternalIdKind::Tmdb,
                        value: format!("person/{n}"),
                    }],
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(metadata::MetadataPayload {
            title: format!("Fetched {}", id.value),
            external_ids: vec![id],
            sort_title: None,
            original_title: None,
            year: Some(2001),
            release_date: None,
            overview: Some("from fixture".into()),
            tagline: None,
            runtime_ms: None,
            genres: vec!["Drama".into()],
            studios: Vec::new(),
            people,
            images,
            rating: None,
            ratings: Vec::new(),
            original_language: None,
            origin_countries: Vec::new(),
            keywords: Vec::new(),
            budget_usd: None,
        })
    }

    async fn fetch_children(
        _parent: ExternalId,
        _parent_work: metadata::WorkId,
    ) -> Result<Vec<metadata::MetadataPayload>, PluginError> {
        Ok(Vec::new())
    }
}

/// Whether the fixture has art of `kind` at all.
fn serves(kind: ImageKind) -> bool {
    matches!(
        kind,
        ImageKind::Primary | ImageKind::Backdrop | ImageKind::Thumb
    )
}

fn kind_name(kind: ImageKind) -> &'static str {
    match kind {
        ImageKind::Primary => "primary",
        ImageKind::Backdrop => "backdrop",
        _ => "thumb",
    }
}

fn png_stream() -> wit_bindgen::StreamReader<u8> {
    let (mut writer, reader) = wit_stream::new::<u8>();
    // The host reads only after the export returns, so write from a task.
    wit_bindgen::spawn_local(async move {
        writer.write_all(PNG.to_vec()).await;
    });
    reader
}

impl image::Guest for Fixture {
    async fn fetch(
        id: ExternalId,
        kind: ImageKind,
        language: Option<String>,
    ) -> Result<(image::ImageMeta, wit_bindgen::StreamReader<u8>), PluginError> {
        if id.value.contains("noart") || !serves(kind) {
            return Err(PluginError::NotFound);
        }
        let language = match language.as_deref() {
            Some("xx") => Some("en".to_string()),
            _ if kind == ImageKind::Backdrop => None,
            _ => language,
        };
        Ok((
            image::ImageMeta {
                kind,
                language,
                mime: "image/png".into(),
                width: Some(1),
                height: Some(1),
            },
            png_stream(),
        ))
    }

    async fn list_candidates(
        id: ExternalId,
        kind: ImageKind,
    ) -> Result<Vec<image::ImageCandidate>, PluginError> {
        if id.value.contains("fail") {
            return Err(PluginError::Unavailable("fixture: list fails".into()));
        }
        if id.value.contains("noart") || !serves(kind) {
            return Ok(Vec::new());
        }
        Ok((1..=2)
            .map(|n| image::ImageCandidate {
                handle: format!("{}|{}|{n}", id.value, kind_name(kind)),
                kind,
                language: (n == 1).then(|| "en".to_string()),
                width: Some(2),
                height: Some(2),
                score: Some(1.0 / n as f32),
            })
            .collect())
    }

    async fn fetch_candidate(
        handle: String,
        preview: bool,
    ) -> Result<(image::ImageMeta, wit_bindgen::StreamReader<u8>), PluginError> {
        if handle == "missing" {
            return Err(PluginError::NotFound);
        }
        let kind = match handle.split('|').nth(1) {
            Some("backdrop") => ImageKind::Backdrop,
            Some("thumb") => ImageKind::Thumb,
            _ => ImageKind::Primary,
        };
        let size = if preview { 1 } else { 2 };
        Ok((
            image::ImageMeta {
                kind,
                language: None,
                mime: "image/png".into(),
                width: Some(size),
                height: Some(size),
            },
            png_stream(),
        ))
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        _task: String,
        _cursor: Option<String>,
        _payload: Option<String>,
    ) -> Result<background::TaskOutcome, PluginError> {
        Err(PluginError::BadInput("fixture: no background tasks".into()))
    }
}

export!(Fixture);
