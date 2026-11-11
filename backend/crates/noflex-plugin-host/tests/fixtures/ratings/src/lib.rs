// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for the host's ratings fan-out, standing in for an
// OMDb-like provider: it rates a work only through an IMDb id, answering
// IMDb 8.0/10 (100 votes) and Rotten Tomatoes 91%, plus two ratings the host
// must drop (a source that isn't a slug, and IMDb above its scale) when the
// id contains `junk`. Ids containing `limited` are rate-limited and ids
// containing `broken` fail, whatever else they name.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "ratings-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{background, ratings};
use noflex::plugin::types::{ExternalId, ExternalIdKind, PluginError, Rating, WorkKind};

struct Fixture;

fn rating(source: &str, value: f32, scale: f32, votes: Option<u64>) -> Rating {
    Rating {
        source: source.into(),
        value,
        scale,
        votes,
    }
}

impl ratings::Guest for Fixture {
    fn attribution() -> Option<ratings::ProviderAttribution> {
        Some(ratings::ProviderAttribution {
            text: "Ratings from the fixture.".into(),
            url: Some("https://ratings.fixture.example".into()),
            logo: None,
        })
    }

    async fn fetch(ids: Vec<ExternalId>, _kind: WorkKind) -> Result<Vec<Rating>, PluginError> {
        if ids.iter().any(|id| id.value.contains("limited")) {
            return Err(PluginError::RateLimited(60));
        }
        if ids.iter().any(|id| id.value.contains("broken")) {
            return Err(PluginError::Internal("fixture: broken".into()));
        }
        let Some(imdb) = ids.iter().find(|id| id.kind == ExternalIdKind::Imdb) else {
            return Ok(Vec::new());
        };
        let mut out = vec![
            rating("imdb", 8.0, 10.0, Some(100)),
            rating("rotten-tomatoes", 91.0, 100.0, None),
        ];
        if imdb.value.contains("junk") {
            out.push(rating("Rotten Tomatoes", 50.0, 100.0, None));
            out.insert(0, rating("imdb", 12.0, 10.0, None));
        }
        Ok(out)
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        _cursor: Option<String>,
        _payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        Err(background::PluginError::BadInput(format!(
            "unknown task `{task}`"
        )))
    }
}

export!(Fixture);
