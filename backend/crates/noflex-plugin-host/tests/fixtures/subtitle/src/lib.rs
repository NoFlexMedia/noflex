// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for the host's subtitle search. `search` answers
// every wanted language (English when none) with an id match, plus a hash
// match when the query has a moviehash, plus a Spanish one whether asked
// for or not. The id match's `release` spells out the whole query, so a
// test can check what the host sent. A title of `Formats` offers one
// handle per kind of download instead. `download` answers by the handle's
// prefix: `srt` is a Latin-1 SubRip file, `ass` an ASS one, `txt` an
// unsupported format, `empty` a file with no cues, and `big` one over the
// host's cap.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "subtitle-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{
    background,
    subtitle::{self, SubtitleMatch, SubtitleQuery},
};
use noflex::plugin::types::{ExternalId, PluginError};

/// "Café" in Latin-1, as old SubRip files often are.
const SRT: &[u8] = b"1\r\n00:00:01,000 --> 00:00:02,500\r\nCaf\xe9\r\n";

const ASS: &str = "[Script Info]\nTitle: Fixture\n\n[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,{\\i1}Hello{\\i0}\n";

struct Fixture;

fn ids(ids: &[ExternalId]) -> String {
    ids.iter()
        .map(|id| format!("{:?}={}", id.kind, id.value))
        .collect::<Vec<_>>()
        .join(",")
}

/// The whole query, one field after another.
fn describe(q: &SubtitleQuery) -> String {
    format!(
        "{:?}|{}|{:?}|{}|{}|{:?}|{:?}|{}|{}|{:?}",
        q.kind,
        q.title,
        q.year,
        ids(&q.external_ids),
        ids(&q.series_external_ids),
        q.season,
        q.episode,
        q.file_name,
        q.file_size,
        q.moviehash,
    )
}

fn offer(handle: String, language: &str, hash_match: bool, downloads: u64) -> SubtitleMatch {
    SubtitleMatch {
        format: handle.split(':').next().unwrap_or("srt").to_string(),
        handle,
        language: language.to_string(),
        locale: None,
        release: None,
        hearing_impaired: false,
        forced: false,
        hash_match,
        download_count: Some(downloads),
        score: None,
    }
}

impl subtitle::Guest for Fixture {
    async fn search(query: SubtitleQuery) -> Result<Vec<SubtitleMatch>, PluginError> {
        if query.title == "Formats" {
            return Ok(["srt", "ass", "txt", "empty", "big"]
                .into_iter()
                .map(|kind| offer(format!("{kind}:eng"), "eng", false, 1))
                .collect());
        }
        let wanted = if query.languages.is_empty() {
            vec!["eng".to_string()]
        } else {
            query.languages.clone()
        };
        let mut out = Vec::new();
        for language in &wanted {
            let mut by_ids = offer(format!("srt:{language}:ids"), language, false, 500);
            by_ids.release = Some(describe(&query));
            out.push(by_ids);
            if query.moviehash.is_some() {
                out.push(offer(format!("srt:{language}:hash"), language, true, 10));
            }
        }
        out.push(offer("srt:spa:extra".into(), "spa", false, 9000));
        Ok(out)
    }

    async fn download(
        handle: String,
    ) -> Result<(String, wit_bindgen::StreamReader<u8>), PluginError> {
        let (format, bytes) = match handle.split(':').next() {
            Some("srt") => ("srt", SRT.to_vec()),
            Some("ass") => ("ass", ASS.as_bytes().to_vec()),
            Some("txt") => ("txt", b"just words".to_vec()),
            Some("empty") => ("srt", b"no cues in here\n".to_vec()),
            Some("big") => ("srt", vec![b'x'; 9 * 1024 * 1024]),
            _ => return Err(PluginError::NotFound),
        };
        let (mut writer, reader) = wit_stream::new::<u8>();
        // The host reads only after this returns, so write from a task.
        wit_bindgen::spawn_local(async move {
            writer.write_all(bytes).await;
        });
        Ok((format.to_string(), reader))
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        _cursor: Option<String>,
        _payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        Err(background::PluginError::BadInput(format!(
            "fixture-subtitle: unknown task `{task}`"
        )))
    }
}

export!(Fixture);
