// SPDX-License-Identifier: AGPL-3.0-only
//
// TheTVDB metadata + image provider — first-party WASM plugin.
// See `../noflex-metadata-tmdb/src/lib.rs` for the same pattern with
// inline commentary. This file is intentionally a near-copy: per-provider
// quirks (auth handshake, paginated season endpoints) land in followups.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../crates/noflex-plugin-wit/wit",
    world: "metadata-image-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{background, image, metadata};

struct Tvdb;

impl metadata::Guest for Tvdb {
    fn attribution() -> Option<metadata::ProviderAttribution> {
        None
    }

    fn supports(kind: metadata::WorkKind) -> bool {
        matches!(
            kind,
            metadata::WorkKind::Series | metadata::WorkKind::Season | metadata::WorkKind::Episode,
        )
    }

    async fn search(
        _query: metadata::SearchQuery,
    ) -> Result<Vec<metadata::MetadataMatch>, metadata::PluginError> {
        Ok(Vec::new())
    }

    async fn fetch(
        _id: metadata::ExternalId,
    ) -> Result<metadata::MetadataPayload, metadata::PluginError> {
        Err(metadata::PluginError::Unavailable(
            "tvdb metadata fetch not yet wired".into(),
        ))
    }

    async fn fetch_children(
        _parent: metadata::ExternalId,
        _parent_work: metadata::WorkId,
    ) -> Result<Vec<metadata::MetadataPayload>, metadata::PluginError> {
        Ok(Vec::new())
    }
}

impl image::Guest for Tvdb {
    async fn fetch(
        _id: image::ExternalId,
        _kind: image::ImageKind,
        _language: Option<String>,
    ) -> Result<(image::ImageMeta, wit_bindgen::StreamReader<u8>), image::PluginError> {
        Err(image::PluginError::Unavailable(
            "tvdb image fetch not yet wired".into(),
        ))
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
