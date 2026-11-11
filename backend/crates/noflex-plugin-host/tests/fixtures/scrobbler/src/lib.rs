// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for noflex-plugin-host's scrobble-dispatcher
// suite. `scrobbler::scrobble` appends a one-line encoding of each
// received event to an instance-global log; the host reads the log back
// through the `report` background task (the established observation
// channel — same trick fixture-notifier uses). Every scrobble also
// exercises the per-user secret imports end-to-end: the guest reads the
// acting user's `access_token`, writes back a rotated value, and
// records both outcomes in the log line, so host tests assert the
// `read-for-user`/`write-for-user` round-trip from storage assertions
// and the log alike. `user-link` mirrors fixture-user-link's canned
// challenge/secrets so the linking surface is covered on the same
// component.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "scrobbler-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use std::sync::Mutex;

use exports::noflex::plugin::{background, scrobbler, user_link};
use noflex::plugin::{
    host_secrets,
    types::{ExternalIdKind, PluginError, ScrobbleAction, ScrobbleEvent, WorkKind},
};

static RECEIVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The state `link-start` issues; `link-complete` rejects anything else.
const FIXTURE_STATE: &str = "fixture-scrobbler-state";

struct Fixture;

fn kind_str(kind: &WorkKind) -> &'static str {
    match kind {
        WorkKind::Movie => "movie",
        WorkKind::Series => "series",
        WorkKind::Season => "season",
        WorkKind::Episode => "episode",
        WorkKind::Album => "album",
        WorkKind::Track => "track",
        WorkKind::Artist => "artist",
        WorkKind::AudioBook => "audio-book",
        WorkKind::Photo => "photo",
    }
}

fn id_kind_str(kind: &ExternalIdKind) -> &'static str {
    match kind {
        ExternalIdKind::Tmdb => "tmdb",
        ExternalIdKind::Tvdb => "tvdb",
        ExternalIdKind::Imdb => "imdb",
        ExternalIdKind::MusicBrainzReleaseGroup => "musicbrainz-release-group",
        ExternalIdKind::MusicBrainzArtist => "musicbrainz-artist",
        ExternalIdKind::MusicBrainzRecording => "musicbrainz-recording",
        ExternalIdKind::AniDb => "anidb",
        ExternalIdKind::Custom => "custom",
    }
}

fn action_str(action: &ScrobbleAction) -> &'static str {
    match action {
        ScrobbleAction::Started => "started",
        ScrobbleAction::Paused => "paused",
        ScrobbleAction::Stopped => "stopped",
    }
}

fn err_kind(err: &PluginError) -> &'static str {
    match err {
        PluginError::CapabilityDenied(_) => "capability-denied",
        PluginError::Unavailable(_) => "unavailable",
        PluginError::BadInput(_) => "bad-input",
        PluginError::RateLimited(_) => "rate-limited",
        _ => "other",
    }
}

/// Flat, assertion-friendly one-line encoding of a scrobble event,
/// suffixed with the outcomes of the read/rotate secret round-trip.
fn encode(event: &ScrobbleEvent, token_read: &str, token_write: &str) -> String {
    let ids = |ids: &[noflex::plugin::types::ExternalId]| {
        ids.iter()
            .map(|id| format!("{}={}", id_kind_str(&id.kind), id.value))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "scrobble|user={}|work={}|kind={}|title={}|year={}|ids={}|series={}|season={}|episode={}|action={}|pos={}|runtime={}|read={}|write={}",
        event.user,
        event.work,
        kind_str(&event.kind),
        event.title,
        event.year.map(|y| y.to_string()).unwrap_or_default(),
        ids(&event.external_ids),
        ids(&event.series_external_ids),
        event
            .season_number
            .map(|n| n.to_string())
            .unwrap_or_default(),
        event
            .episode_number
            .map(|n| n.to_string())
            .unwrap_or_default(),
        action_str(&event.action),
        event.position_ms.map(|p| p.to_string()).unwrap_or_default(),
        event.runtime_ms.map(|r| r.to_string()).unwrap_or_default(),
        token_read,
        token_write,
    )
}

impl scrobbler::Guest for Fixture {
    async fn scrobble(event: ScrobbleEvent) -> Result<(), PluginError> {
        // Read the acting user's token, then persist a rotated value —
        // the host test asserts the stored secret changed.
        let read =
            match host_secrets::read_for_user(event.user.clone(), "access_token".into()).await {
                Ok(value) => {
                    let rotated = format!("{value}-rotated");
                    let write = match host_secrets::write_for_user(
                        event.user.clone(),
                        "access_token".into(),
                        rotated,
                    )
                    .await
                    {
                        Ok(()) => "ok".to_string(),
                        Err(e) => format!("err:{}", err_kind(&e)),
                    };
                    RECEIVED.lock().expect("received log poisoned").push(encode(
                        &event,
                        &format!("ok:{value}"),
                        &write,
                    ));
                    return Ok(());
                }
                Err(e) => format!("err:{}", err_kind(&e)),
            };
        RECEIVED
            .lock()
            .expect("received log poisoned")
            .push(encode(&event, &read, "skipped"));
        Ok(())
    }
}

impl user_link::Guest for Fixture {
    async fn link_start(user: String) -> Result<user_link::LinkChallenge, PluginError> {
        Ok(user_link::LinkChallenge {
            auth_url: format!("https://example.invalid/authorize?user={user}"),
            state: FIXTURE_STATE.into(),
            user_code: None,
        })
    }

    async fn unlink(_user: String) -> Result<(), PluginError> {
        Ok(())
    }

    async fn link_complete(
        user: String,
        state: String,
        callback_query: String,
    ) -> Result<Vec<user_link::UserSecret>, PluginError> {
        let _ = (user, callback_query);
        if state != FIXTURE_STATE {
            return Err(PluginError::BadInput(format!(
                "fixture-scrobbler: unexpected state `{state}`"
            )));
        }
        Ok(vec![user_link::UserSecret {
            key: "access_token".into(),
            value: "fixture-linked-token".into(),
        }])
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        _cursor: Option<String>,
        _payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        match task.as_str() {
            // Hand the whole log back as the cursor; `more` is just the
            // transport, the host test never re-invokes.
            "report" => Ok(background::TaskOutcome::More(
                RECEIVED.lock().expect("received log poisoned").join("\n"),
            )),
            other => Err(background::PluginError::BadInput(format!(
                "fixture-scrobbler: unknown task `{other}`"
            ))),
        }
    }
}

export!(Fixture);
