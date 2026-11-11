// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for noflex-plugin-host's one-instance concurrency
// test. Not a shipped provider: `metadata::search` tracks how many calls
// are in flight inside this guest and reports the high-water mark back to
// the host, which lets the test prove that N concurrent hook calls really
// overlapped on the *same* instance (per-call instantiation would pin the
// mark at 1). `image::fetch` returns a small payload over a real
// `stream<u8>` so the host's stream-to-buffer ingestion is exercised too.

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

use std::cell::Cell;

use exports::noflex::plugin::{background, image, metadata};
use noflex::plugin::types::ExternalIdKind;

/// Deterministic fake image payload the host test asserts byte-for-byte.
const IMAGE_BYTES: &[u8] = b"noflex-fixture-image-bytes";

thread_local! {
    static IN_FLIGHT: Cell<u32> = const { Cell::new(0) };
    static MAX_IN_FLIGHT: Cell<u32> = const { Cell::new(0) };
}

struct Fixture;

impl metadata::Guest for Fixture {
    fn attribution() -> Option<metadata::ProviderAttribution> {
        None
    }

    fn supports(_kind: metadata::WorkKind) -> bool {
        true
    }

    async fn search(
        query: metadata::SearchQuery,
    ) -> Result<Vec<metadata::MetadataMatch>, metadata::PluginError> {
        let n = IN_FLIGHT.with(|c| {
            let n = c.get() + 1;
            c.set(n);
            n
        });
        MAX_IN_FLIGHT.with(|c| c.set(c.get().max(n)));
        // Park this task repeatedly so the host event loop can enter the
        // instance with the next concurrent call.
        for _ in 0..20 {
            wit_bindgen::yield_async().await;
        }
        IN_FLIGHT.with(|c| c.set(c.get() - 1));
        let max = MAX_IN_FLIGHT.with(|c| c.get());
        Ok(vec![metadata::MetadataMatch {
            external_id: metadata::ExternalId {
                kind: ExternalIdKind::Custom,
                value: format!("max-in-flight:{max}"),
            },
            // Echo the title so the host can verify each caller got its
            // own response, not a sibling's.
            title: query.title,
            year: None,
            confidence: 1.0,
        }])
    }

    /// Deterministic payload the host test asserts on; the requested id is
    /// echoed back in `external_ids` so the host can verify the round-trip.
    async fn fetch(
        id: metadata::ExternalId,
    ) -> Result<metadata::MetadataPayload, metadata::PluginError> {
        use noflex::plugin::types::{ImageKind, MetadataImage, MetadataPerson, Rating};
        Ok(metadata::MetadataPayload {
            external_ids: vec![id],
            title: "Fixture Title".into(),
            sort_title: Some("Fixture Title".into()),
            original_title: Some("Fixture Title (original)".into()),
            year: Some(2001),
            release_date: Some("2001-05-04".into()),
            overview: Some("A fixture overview.".into()),
            tagline: Some("A fixture tagline.".into()),
            runtime_ms: Some(5_420_000),
            genres: vec!["Drama".into(), "Fixture".into()],
            studios: vec!["Fixture Studios".into()],
            people: vec![MetadataPerson {
                name: "Fixture Director".into(),
                role: Some("Director".into()),
                character: None,
                external_ids: vec![],
            }],
            images: vec![MetadataImage {
                kind: ImageKind::Primary,
                url: "https://example.invalid/poster.jpg".into(),
                language: None,
                width: None,
                height: None,
            }],
            rating: Some("PG-13".into()),
            ratings: vec![Rating {
                source: "tmdb".into(),
                value: 7.5,
                scale: 10.0,
                votes: Some(1200),
            }],
            original_language: None,
            origin_countries: Vec::new(),
            keywords: Vec::new(),
            budget_usd: None,
        })
    }

    async fn fetch_children(
        _parent: metadata::ExternalId,
        _parent_work: metadata::WorkId,
    ) -> Result<Vec<metadata::MetadataPayload>, metadata::PluginError> {
        Ok(Vec::new())
    }
}

impl image::Guest for Fixture {
    async fn fetch(
        _id: image::ExternalId,
        kind: image::ImageKind,
        _language: Option<String>,
    ) -> Result<(image::ImageMeta, wit_bindgen::StreamReader<u8>), image::PluginError> {
        let (mut writer, reader) = wit_stream::new::<u8>();
        // Writing must not be awaited inline: the host only starts reading
        // after this function returns the read end, so a backpressured
        // write would deadlock the export task.
        wit_bindgen::spawn_local(async move {
            writer.write_all(IMAGE_BYTES.to_vec()).await;
        });
        Ok((
            image::ImageMeta {
                kind,
                language: None,
                mime: "application/octet-stream".into(),
                width: None,
                height: None,
            },
            reader,
        ))
    }

    async fn list_candidates(
        _id: image::ExternalId,
        _kind: image::ImageKind,
    ) -> Result<Vec<image::ImageCandidate>, image::PluginError> {
        Ok(Vec::new())
    }

    async fn fetch_candidate(
        _handle: String,
        _preview: bool,
    ) -> Result<(image::ImageMeta, wit_bindgen::StreamReader<u8>), image::PluginError> {
        Err(image::PluginError::NotFound)
    }
}

// -------- Background hook (stub — this fixture registers no [[tasks]]) --------

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        let _ = (task, cursor, payload);
        Err(background::PluginError::BadInput(
            "fixture: no background tasks".into(),
        ))
    }
}

export!(Fixture);
