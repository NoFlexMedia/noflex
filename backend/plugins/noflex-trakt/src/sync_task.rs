// SPDX-License-Identifier: AGPL-3.0-only
//
// The `sync` background task: two-way history and ratings sync for one
// user, a page per run. The host starts it for each linked user on its
// schedule (a `per_user` task, the user's id as the payload), and the
// device-link poll enqueues it the moment someone links. Each run does one
// page of work and answers `more(cursor)` until the plan in `crate::sync`
// is done, then stores what it saw as the user's `sync_state` secret.

use crate::{
    api::{self, IdKind, MediaKind},
    guest::{
        exports::noflex::plugin::background::TaskOutcome,
        log,
        noflex::plugin::{
            host_library, host_log, host_net, host_secrets, host_user_state,
            types::{ExternalId, ExternalIdKind, PluginError, WorkKind},
        },
        refresh_access_token,
    },
    sync::{self, Local, Step, SyncCursor, SyncState, Target},
};

const STATE_KEY: &str = "sync_state";
const NANOS_PER_SEC: u64 = 1_000_000_000;
const TIMEOUT_MS: u32 = 30_000;
/// Longest a 429 is waited out within a run; anything longer is handed
/// back as `rate-limited`, and the chain tries again at the next tick.
const MAX_INLINE_WAIT_SECS: u32 = 20;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

async fn wait_secs(secs: u32) {
    crate::guest::wasi::clocks::monotonic_clock::wait_for(u64::from(secs) * NANOS_PER_SEC).await;
}

/// One user's authenticated session with Trakt.
struct Session {
    user: String,
    client_id: String,
    token: String,
}

impl Session {
    async fn open(user: &str) -> Result<Self, PluginError> {
        let token = host_secrets::read_for_user(user.into(), "access_token".into()).await?;
        let client_id = host_secrets::read("client_id".into()).await?;
        Ok(Self {
            user: user.into(),
            client_id,
            token,
        })
    }

    /// `method` on `path` under the API, refreshing the token once on a 401
    /// and waiting out one short 429. POSTs are paced to Trakt's limit of
    /// one a second per user.
    async fn send(
        &mut self,
        method: &str,
        path_and_query: &str,
        body: Option<String>,
    ) -> Result<host_net::HttpResponse, PluginError> {
        let mut refreshed = false;
        let mut waited = false;
        loop {
            if method == "POST" {
                wait_secs(1).await;
            }
            let response = host_net::request(host_net::HttpRequest {
                method: method.into(),
                url: format!("{}{path_and_query}", api::API_BASE),
                headers: api::api_headers(&self.client_id, Some(&self.token)),
                body: body.clone().map(String::into_bytes),
                timeout_ms: Some(TIMEOUT_MS),
            })
            .await?;
            let path = path_and_query.split('?').next().unwrap_or_default();
            match response.status {
                200..=299 => return Ok(response),
                401 if !refreshed => {
                    refreshed = true;
                    self.token = refresh_access_token(&self.user, &self.client_id)
                        .await
                        .ok_or_else(|| {
                            PluginError::Unavailable("trakt: token refresh failed".into())
                        })?;
                }
                429 => {
                    let secs = sync::retry_after(&response.headers);
                    if waited || secs > MAX_INLINE_WAIT_SECS {
                        return Err(PluginError::RateLimited(secs));
                    }
                    waited = true;
                    wait_secs(secs).await;
                }
                status => {
                    return Err(PluginError::Unavailable(format!(
                        "trakt: {method} {path} answered {status}"
                    )));
                }
            }
        }
    }
}

/// Run one page of a user's sync. `cursor` continues a chain; otherwise
/// `payload` names the user a new one starts for.
pub(crate) async fn run(
    cursor: Option<String>,
    payload: Option<String>,
) -> Result<TaskOutcome, PluginError> {
    let (mut sync, mut session) = match cursor {
        Some(raw) => {
            let Some(sync) = SyncCursor::decode(&raw) else {
                log(
                    host_log::Level::Warn,
                    "trakt: dropping an unreadable sync cursor",
                );
                return Ok(TaskOutcome::Done);
            };
            let session = Session::open(&sync.user).await?;
            (sync, session)
        }
        None => {
            let Some(user) = payload.filter(|user| !user.trim().is_empty()) else {
                return Ok(TaskOutcome::Done);
            };
            match start(&user).await {
                Ok(started) => started,
                // An unbound run (an administrator's "Run now") acts for nobody.
                Err(PluginError::Unavailable(reason))
                | Err(PluginError::CapabilityDenied(reason)) => {
                    log(
                        host_log::Level::Info,
                        &format!("trakt: no sync for `{user}`: {reason}"),
                    );
                    return Ok(TaskOutcome::Done);
                }
                Err(e) => return Err(e),
            }
        }
    };
    step(&mut sync, &mut session).await?;
    if !sync.finished() {
        return Ok(TaskOutcome::More(sync.encode()));
    }
    host_secrets::write_for_user(sync.user.clone(), STATE_KEY.into(), sync.state().encode())
        .await?;
    Ok(TaskOutcome::Done)
}

/// Plan a sync for `user` from their last one and Trakt's activity times:
/// the one request an idle account costs.
async fn start(user: &str) -> Result<(SyncCursor, Session), PluginError> {
    let mut session = Session::open(user).await?;
    let state = host_secrets::read_for_user(user.into(), STATE_KEY.into())
        .await
        .ok()
        .and_then(|raw| SyncState::decode(&raw));
    let response = session.send("GET", "/sync/last_activities", None).await?;
    let latest = sync::parse_last_activities(&response.body).map_err(PluginError::Internal)?;
    Ok((
        SyncCursor::plan(user.into(), now_ms(), state.as_ref(), latest),
        session,
    ))
}

async fn step(sync: &mut SyncCursor, session: &mut Session) -> Result<(), PluginError> {
    let Some(current) = sync.steps.first().copied() else {
        return Ok(());
    };
    if current == Step::Export {
        return export(sync, session).await;
    }
    let url = sync.page_url("").unwrap_or_default();
    let response = session.send("GET", &url, None).await?;
    let items = match current {
        Step::HistoryMovies | Step::HistoryEpisodes => {
            let mut items = Vec::new();
            for watched in sync::parse_history(&response.body).map_err(PluginError::Internal)? {
                for work in resolve(&watched.target).await {
                    items.push(host_user_state::StateImport {
                        work,
                        played_at: Some(watched.watched_at),
                        play_count: Some(watched.plays),
                        rating: None,
                        favorite: false,
                    });
                }
            }
            items
        }
        _ => {
            let mut items = Vec::new();
            for rated in sync::parse_ratings(&response.body).map_err(PluginError::Internal)? {
                for work in resolve(&rated.target).await {
                    items.push(host_user_state::StateImport {
                        work,
                        played_at: None,
                        play_count: None,
                        rating: Some(host_user_state::UserRating {
                            value: f32::from(rated.rating),
                            rated_at: rated.rated_at,
                        }),
                        favorite: false,
                    });
                }
            }
            items
        }
    };
    if !items.is_empty() {
        host_user_state::import_user_state(sync.user.clone(), items).await?;
    }
    sync.advance(sync::page_count(&response.headers));
    Ok(())
}

/// Send Trakt one page of what this server has: plays and ratings since
/// the window opened, or everything on a full sync. The host has already
/// left out what this plugin imported.
async fn export(sync: &mut SyncCursor, session: &mut Session) -> Result<(), PluginError> {
    let page =
        host_user_state::user_history(sync.user.clone(), sync.since, sync.export_cursor.clone())
            .await?;
    let since = sync.since.unwrap_or(i64::MIN);
    let locals = page
        .entries
        .iter()
        .map(|entry| Local {
            kind: match entry.kind {
                WorkKind::Movie => MediaKind::Movie,
                WorkKind::Series => MediaKind::Show,
                WorkKind::Episode => MediaKind::Episode,
                _ => MediaKind::Other,
            },
            ids: id_pairs(&entry.external_ids),
            show_ids: id_pairs(&entry.series_external_ids),
            season: entry.season_number,
            number: entry.episode_number,
            // An entry is listed for whichever part changed; send only that.
            played_at: entry.played_at.filter(|at| *at >= since),
            rating: entry
                .rating
                .as_ref()
                .filter(|rating| rating.rated_at >= since)
                .map(|rating| (rating.value, rating.rated_at)),
        })
        .collect::<Vec<_>>();
    if let Some(body) = sync::history_body(&locals) {
        session.send("POST", "/sync/history", Some(body)).await?;
    }
    if let Some(body) = sync::ratings_body(&locals) {
        session.send("POST", "/sync/ratings", Some(body)).await?;
    }
    match page.next_cursor {
        Some(next) => sync.export_cursor = Some(next),
        None => sync.next_step(),
    }
    Ok(())
}

fn id_pairs(ids: &[ExternalId]) -> Vec<(IdKind, String)> {
    ids.iter()
        .filter_map(|id| {
            let kind = match id.kind {
                ExternalIdKind::Tmdb => IdKind::Tmdb,
                ExternalIdKind::Tvdb => IdKind::Tvdb,
                ExternalIdKind::Imdb => IdKind::Imdb,
                _ => return None,
            };
            Some((kind, id.value.clone()))
        })
        .collect()
}

fn wit_ids(ids: Vec<(IdKind, String)>) -> Vec<ExternalId> {
    ids.into_iter()
        .filter_map(|(kind, value)| {
            let kind = match kind {
                IdKind::Tmdb => ExternalIdKind::Tmdb,
                IdKind::Tvdb => ExternalIdKind::Tvdb,
                IdKind::Imdb => ExternalIdKind::Imdb,
                IdKind::Other => return None,
            };
            Some(ExternalId { kind, value })
        })
        .collect()
}

/// The works here a Trakt item is: by its ids, a movie or show by the first
/// id that finds one of that kind, an episode by its show's ids and number.
async fn resolve(target: &Target) -> Vec<String> {
    let (ids, kind) = match target {
        Target::Movie(ids) => (ids, WorkKind::Movie),
        Target::Show(ids) => (ids, WorkKind::Series),
        Target::Episode {
            show,
            season,
            number,
        } => {
            let series = wit_ids(show.lookups(MediaKind::Show));
            return host_library::find_episode(series, *season, *number)
                .await
                .map(|work| vec![work.id])
                .unwrap_or_default();
        }
    };
    let media = if kind == WorkKind::Movie {
        MediaKind::Movie
    } else {
        MediaKind::Show
    };
    for id in wit_ids(ids.lookups(media)) {
        let works = host_library::find_works(id)
            .await
            .into_iter()
            .filter(|work| work.kind == kind)
            .map(|work| work.id)
            .collect::<Vec<_>>();
        if !works.is_empty() {
            return works;
        }
    }
    Vec::new()
}
