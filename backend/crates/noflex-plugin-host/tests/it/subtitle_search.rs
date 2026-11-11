// SPDX-License-Identifier: AGPL-3.0-only
//
// The `subtitle` hook across plugins: a search reaches every subtitle
// plugin with the whole query, keeps one provider's results when another
// fails, and ranks hash matches first; a download comes back as its format
// and bytes, capped. The fixture guests (`tests/fixtures/subtitle/` and
// `subtitle-quota/`) are built on demand; the tests self-skip if the
// toolchain can't produce them.

use noflex_core::{
    model::{ExternalId, ExternalIdKind, WorkKind},
    providers::SubtitleQuery,
};
use noflex_plugin_host::{EngineConfig, PluginRegistry, ProviderError, build_engine};

fn registry(fixtures: &[&str]) -> Option<PluginRegistry> {
    let engine = build_engine(&EngineConfig::default()).expect("engine builds");
    let registry = PluginRegistry::new(engine).expect("registry");
    for fixture in fixtures {
        let wasm = crate::fixtures::wasm(fixture)?;
        registry
            .load_from(&wasm)
            .unwrap_or_else(|e| panic!("load {fixture} fixture: {e}"));
    }
    Some(registry)
}

fn episode_query(languages: &[&str]) -> SubtitleQuery {
    SubtitleQuery {
        moviehash: Some("8e245d9679d31e12".into()),
        file_size: 12_909_756,
        file_name: "Show.S02E06.mkv".into(),
        title: "A Fractured House".into(),
        year: Some(2014),
        kind: WorkKind::Episode,
        external_ids: vec![
            ExternalId {
                kind: ExternalIdKind::Imdb,
                value: "tt4078580".into(),
            },
            ExternalId {
                kind: ExternalIdKind::Custom,
                value: "/media/Shows/Show/Season 2".into(),
            },
        ],
        series_external_ids: vec![ExternalId {
            kind: ExternalIdKind::Tmdb,
            value: "1403".into(),
        }],
        season: Some(2),
        episode: Some(6),
        languages: languages.iter().map(|l| l.to_string()).collect(),
    }
}

#[tokio::test]
async fn one_provider_failing_hides_nothing_the_others_found() {
    let Some(registry) = registry(&["subtitle", "subtitle-quota"]) else {
        eprintln!("skipping: could not build the subtitle fixtures");
        return;
    };
    // `en` and `ger` are spellings of languages the host canonicalizes.
    let search = registry
        .search_subtitles(&episode_query(&["en", "ger"]))
        .await;

    assert_eq!(search.failures.len(), 1, "{:?}", search.failures);
    assert_eq!(search.failures[0].provider, "fixture-subtitle-quota");
    assert_eq!(
        search.failures[0].error,
        ProviderError::RateLimited {
            retry_after_secs: 3600
        }
    );

    let found: Vec<_> = search
        .matches
        .iter()
        .map(|m| (m.handle.as_str(), m.language.as_str(), m.hash_match))
        .collect();
    assert_eq!(
        found,
        [
            ("srt:deu:hash", "deu", true),
            ("srt:eng:hash", "eng", true),
            ("srt:deu:ids", "deu", false),
            ("srt:eng:ids", "eng", false),
        ],
        "hash matches first, and the unasked-for Spanish one is left out"
    );
    assert!(
        search
            .matches
            .iter()
            .all(|m| m.provider == "fixture-subtitle")
    );
}

#[tokio::test]
async fn the_whole_query_reaches_the_plugin_without_scanner_keys() {
    let Some(registry) = registry(&["subtitle"]) else {
        eprintln!("skipping: could not build the subtitle fixture");
        return;
    };
    let search = registry.search_subtitles(&episode_query(&["eng"])).await;
    let echoed = search
        .matches
        .iter()
        .find(|m| m.handle == "srt:eng:ids")
        .and_then(|m| m.release.clone())
        .expect("the id match spells out the query");
    assert_eq!(
        echoed,
        "WorkKind::Episode|A Fractured House|Some(2014)|ExternalIdKind::Imdb=tt4078580|\
         ExternalIdKind::Tmdb=1403|Some(2)|Some(6)|Show.S02E06.mkv|12909756|\
         Some(\"8e245d9679d31e12\")"
    );
}

#[tokio::test]
async fn no_languages_asked_for_means_any() {
    let Some(registry) = registry(&["subtitle"]) else {
        eprintln!("skipping: could not build the subtitle fixture");
        return;
    };
    let search = registry.search_subtitles(&episode_query(&[])).await;
    assert!(search.failures.is_empty());
    assert!(
        search.matches.iter().any(|m| m.language == "spa"),
        "Spanish, which the fixture always offers"
    );
}

#[tokio::test]
async fn downloads_come_back_with_their_format_and_within_the_cap() {
    let Some(registry) = registry(&["subtitle", "subtitle-quota"]) else {
        eprintln!("skipping: could not build the subtitle fixtures");
        return;
    };
    let file = registry
        .download_subtitle("fixture-subtitle", "srt:eng:ids")
        .await
        .expect("download");
    assert_eq!(file.format, "srt");
    assert_eq!(
        &file.bytes[..],
        b"1\r\n00:00:01,000 --> 00:00:02,500\r\nCaf\xe9\r\n",
        "bytes arrive exactly as the provider sent them"
    );

    let too_big = registry
        .download_subtitle("fixture-subtitle", "big:eng")
        .await
        .expect_err("over the cap");
    assert!(
        matches!(&too_big.error, ProviderError::Failed(m) if m.contains("byte limit")),
        "{too_big:?}"
    );

    let quota = registry
        .download_subtitle("fixture-subtitle-quota", "anything")
        .await
        .expect_err("quota used up");
    assert_eq!(
        quota.error,
        ProviderError::RateLimited {
            retry_after_secs: 3600
        }
    );

    let unknown = registry
        .download_subtitle("not-loaded", "srt:eng:ids")
        .await
        .expect_err("no such provider");
    assert_eq!(unknown.error, ProviderError::NotFound);
}
