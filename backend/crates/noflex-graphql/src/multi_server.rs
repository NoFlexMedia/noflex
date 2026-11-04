// SPDX-License-Identifier: AGPL-3.0-only
//
// What connecting to several servers needs from each one. Servers never talk
// to each other: clients do the merging and carry state between them.

use std::sync::Arc;

use async_graphql::{Context, InputObject, Object, SimpleObject};
use noflex_core::{
    ExternalId, ExternalIdKind, InviteId, LibraryId, LinkedServerId, PlayTarget, PlayTargetKind,
    UserId, UserRole, WorkId, WorkKind,
};
use noflex_storage::{
    Storage,
    domain::{Invite, LinkedServer, User, Work},
    query::PageCursor,
};
use time::{Duration, OffsetDateTime};

use crate::{
    access,
    auth::{LoginResult, login_throttle, mint_session, too_many_attempts},
    users::authed_principal,
};

const MAX_INVITE_DAYS: i64 = 365;
const MAX_SERVER_NAME_CHARS: usize = 100;
/// How much of a user's history one sync call reads.
const SYNC_HISTORY_LIMIT: usize = 500;

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

/// `raw` reduced to the origin a browser would compare, or an error for anything
/// that isn't plain http(s) to a host. The result goes into CSP headers, so the
/// allowed characters are deliberately narrow: nothing here can end a directive.
pub fn normalize_origin(raw: &str) -> Result<String, &'static str> {
    const INVALID: &str = "enter a server address like https://media.example.com";
    let raw = raw.trim().trim_end_matches('/');
    let (scheme, rest) = raw.split_once("://").ok_or(INVALID)?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(INVALID);
    }
    let host = rest.to_ascii_lowercase();
    let valid = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
        && host.chars().next().is_some_and(|c| c != ':' && c != '.');
    if !valid {
        return Err(INVALID);
    }
    Ok(format!("{scheme}://{host}"))
}

/// An invite at the only moment its token exists.
#[derive(SimpleObject, Debug, Clone)]
pub struct CreatedInvite {
    pub invite: Invite,
    /// Goes in the invite link; can't be retrieved later.
    pub token: String,
}

/// One title's watch state, in terms another server can match.
#[derive(SimpleObject, InputObject, Debug, Clone)]
#[graphql(input_name = "WatchSyncItemInput")]
pub struct WatchSyncItem {
    /// Provider IDs of the movie, or of the series for an episode. Paths and
    /// other server-local IDs never appear here, so nothing syncs on a guess.
    pub external_ids: Vec<ExternalId>,
    /// Set for an episode, with `episode`.
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub played: bool,
    pub position_ms: Option<i64>,
    pub last_played: OffsetDateTime,
}

/// The series or movie to match on: `work` itself unless it's an episode, and the
/// season and episode number within it.
async fn sync_identity(
    storage: &Arc<dyn Storage>,
    work: &Work,
) -> async_graphql::Result<Option<(Vec<ExternalId>, Option<(i32, i32)>)>> {
    if work.kind != WorkKind::Episode {
        return Ok(Some((work.provider_ids(), None)));
    }
    let (Some(season), Some(episode)) = (work.parent_index, work.index_number) else {
        return Ok(None);
    };
    let series = work
        .series_of_episode(storage.as_ref())
        .await
        .map_err(gql_err)?;
    Ok(series.map(|series| (series.provider_ids(), Some((season, episode)))))
}

/// The local work an incoming item means, if exactly one provider ID leads to it.
async fn sync_target(
    storage: &Arc<dyn Storage>,
    item: &WatchSyncItem,
) -> async_graphql::Result<Option<WorkId>> {
    for id in item
        .external_ids
        .iter()
        .filter(|id| id.kind != ExternalIdKind::Custom)
    {
        let matches = storage
            .works_by_external_id(id.kind, &id.value)
            .await
            .map_err(gql_err)?;
        let wanted_kinds: &[WorkKind] = if item.season.is_some() {
            &[WorkKind::Series]
        } else {
            &[WorkKind::Movie]
        };
        let [found] = matches
            .iter()
            .filter(|work| wanted_kinds.contains(&work.kind))
            .collect::<Vec<_>>()[..]
        else {
            continue;
        };
        let (Some(season), Some(episode)) = (item.season, item.episode) else {
            return Ok(Some(found.id));
        };
        let next = storage
            .episodes_after(found.id, season, episode - 1, 1)
            .await
            .map_err(gql_err)?
            .into_iter()
            .find(|work| work.parent_index == Some(season) && work.index_number == Some(episode));
        return Ok(next.map(|work| work.id));
    }
    Ok(None)
}

#[derive(Default)]
pub struct MultiServerQuery;

#[Object]
impl MultiServerQuery {
    /// Every invite, newest first. Administrator only.
    async fn invites(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<Invite>> {
        access::require_admin(ctx)?;
        storage(ctx)?.list_invites().await.map_err(gql_err)
    }

    /// The other servers the caller has linked, with the keys their clients
    /// connect with.
    async fn linked_servers(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<LinkedServer>> {
        let principal = authed_principal(ctx)?;
        storage(ctx)?
            .linked_servers_for_user(principal.user_id)
            .await
            .map_err(gql_err)
    }

    /// The caller's watch activity since `since`, newest first, in a form other
    /// servers can match. Titles without a provider ID are left out, since
    /// nothing could safely match them.
    async fn watch_sync_items(
        &self,
        ctx: &Context<'_>,
        since: OffsetDateTime,
    ) -> async_graphql::Result<Vec<WatchSyncItem>> {
        // Boxed because every resolver's future is laid out inside the merged
        // root's, and this one is large enough to overflow a worker's stack.
        Box::pin(watch_sync_items(ctx, since)).await
    }
}

async fn watch_sync_items(
    ctx: &Context<'_>,
    since: OffsetDateTime,
) -> async_graphql::Result<Vec<WatchSyncItem>> {
    let principal = authed_principal(ctx)?;
    let storage = storage(ctx)?;
    let mut out = Vec::new();
    let mut cursor: Option<PageCursor> = None;
    let mut read = 0;
    'pages: while read < SYNC_HISTORY_LIMIT {
        let page = storage
            .list_play_history(principal.user_id, cursor, 100)
            .await
            .map_err(gql_err)?;
        for row in &page.rows {
            read += 1;
            // Newest first, so the first older row ends it.
            let Some(last_played) = row.data.last_played.filter(|at| *at >= since) else {
                break 'pages;
            };
            if row.target_kind != PlayTargetKind::Work {
                continue;
            }
            let work = storage
                .get_work(WorkId::from_uuid(row.target_id))
                .await
                .map_err(gql_err)?;
            let Some(work) = work else { continue };
            let Some((external_ids, episode)) = sync_identity(storage, &work).await? else {
                continue;
            };
            if external_ids.is_empty() {
                continue;
            }
            out.push(WatchSyncItem {
                external_ids,
                season: episode.map(|(season, _)| season),
                episode: episode.map(|(_, episode)| episode),
                played: row.data.played,
                position_ms: row.data.position_ms,
                last_played,
            });
        }
        match page.next_cursor {
            Some(next) => cursor = Some(PageCursor(next)),
            None => break,
        }
    }
    Ok(out)
}

#[derive(Default)]
pub struct MultiServerMutation;

#[Object]
impl MultiServerMutation {
    /// Create an invite that lets someone make an ordinary account with access
    /// to `libraryIds`. Administrator only.
    async fn create_invite(
        &self,
        ctx: &Context<'_>,
        library_ids: Vec<LibraryId>,
        #[graphql(default = 7)] expires_in_days: i32,
        #[graphql(default = 1)] max_uses: i32,
    ) -> async_graphql::Result<CreatedInvite> {
        let admin = access::require_admin(ctx)?;
        let days = i64::from(expires_in_days);
        if !(1..=MAX_INVITE_DAYS).contains(&days) {
            return Err(async_graphql::Error::new(
                "an invite lasts between 1 and 365 days",
            ));
        }
        let max_uses = u32::try_from(max_uses)
            .ok()
            .filter(|uses| *uses > 0)
            .ok_or_else(|| async_graphql::Error::new("an invite needs at least one use"))?;
        let now = OffsetDateTime::now_utc();
        let invite = Invite {
            id: InviteId::new(),
            created_by: admin.user_id,
            library_ids,
            max_uses,
            uses: 0,
            expires_at: now + Duration::days(days),
            revoked_at: None,
            created_at: now,
        };
        let token = noflex_auth::generate_api_key();
        storage(ctx)?
            .create_invite(&invite, &noflex_auth::hash_api_key(&token))
            .await
            .map_err(gql_err)?;
        Ok(CreatedInvite { invite, token })
    }

    /// Stop an invite working. Returns whether it was still active.
    /// Administrator only.
    async fn revoke_invite(&self, ctx: &Context<'_>, id: InviteId) -> async_graphql::Result<bool> {
        access::require_admin(ctx)?;
        storage(ctx)?
            .revoke_invite(id, OffsetDateTime::now_utc())
            .await
            .map_err(gql_err)
    }

    /// Create an account from an invite and sign in. Accounts made this way are
    /// never administrators. Needs no authentication; failures are throttled
    /// like sign-ins.
    async fn redeem_invite(
        &self,
        ctx: &Context<'_>,
        token: String,
        username: String,
        password: String,
        client: Option<String>,
        device: Option<String>,
    ) -> async_graphql::Result<LoginResult> {
        let storage = storage(ctx)?;
        if username.trim().is_empty() || password.len() < 8 {
            return Err(async_graphql::Error::new(
                "choose a username and a password of at least 8 characters",
            ));
        }
        if username.len() > noflex_auth::MAX_USERNAME_BYTES
            || password.len() > noflex_auth::MAX_PASSWORD_BYTES
        {
            return Err(async_graphql::Error::new("credentials are too long"));
        }
        let throttle = login_throttle(ctx)?;
        let client_addr = ctx.data_opt::<noflex_auth::ClientAddr>().copied();
        throttle
            .check("invite", client_addr)
            .map_err(too_many_attempts)?;
        // Before spending a use, so a taken name doesn't burn the invite.
        if storage
            .get_user_by_name(&username)
            .await
            .map_err(gql_err)?
            .is_some()
        {
            return Err(async_graphql::Error::new("that username is taken"));
        }
        let hash = noflex_auth::hash_password(&password)
            .await
            .map_err(|_| async_graphql::Error::new("password processing failed"))?;
        let now = OffsetDateTime::now_utc();
        let Some(invite) = storage
            .consume_invite(&noflex_auth::hash_api_key(&token), now)
            .await
            .map_err(gql_err)?
        else {
            throttle.record_failure("invite", client_addr);
            return Err(async_graphql::Error::new(
                "this invite has expired or already been used",
            ));
        };
        throttle.record_success("invite");
        let user = User {
            id: UserId::new(),
            name: username,
            password_hash: hash,
            is_admin: false,
            role: UserRole::User,
            created_at: now,
        };
        if storage.upsert_user(&user).await.is_err() {
            // Lost a race for the name; the use wasn't really spent.
            storage
                .release_invite_use(invite.id)
                .await
                .map_err(gql_err)?;
            return Err(async_graphql::Error::new("that username is taken"));
        }
        for library in &invite.library_ids {
            storage
                .grant_library_access(user.id, *library)
                .await
                .map_err(gql_err)?;
        }
        let (token, refresh_token) = mint_session(ctx, &user, client, device).await?;
        Ok(LoginResult {
            token,
            refresh_token,
            user,
        })
    }

    /// Link another server, or replace the key of one already linked. An empty
    /// `apiKey` saves just the address. Web clients need that first: their
    /// security policy only allows origins known when the page loaded, so they
    /// reload before they can sign in there and link again with a real key.
    async fn add_linked_server(
        &self,
        ctx: &Context<'_>,
        origin: String,
        name: String,
        #[graphql(default)] api_key: String,
    ) -> async_graphql::Result<LinkedServer> {
        let principal = authed_principal(ctx)?;
        let origin = normalize_origin(&origin).map_err(async_graphql::Error::new)?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > MAX_SERVER_NAME_CHARS {
            return Err(async_graphql::Error::new(
                "a server name must be 1 to 100 characters",
            ));
        }
        storage(ctx)?
            .upsert_linked_server(&LinkedServer {
                id: LinkedServerId::new(),
                user_id: principal.user_id,
                origin,
                name: name.to_string(),
                api_key,
                sync_enabled: true,
                synced_until: None,
                created_at: OffsetDateTime::now_utc(),
            })
            .await
            .map_err(gql_err)
    }

    /// Forget a linked server. Returns whether the caller had it. The key stays
    /// valid on that server until revoked there.
    async fn remove_linked_server(
        &self,
        ctx: &Context<'_>,
        id: LinkedServerId,
    ) -> async_graphql::Result<bool> {
        let principal = authed_principal(ctx)?;
        storage(ctx)?
            .delete_linked_server(principal.user_id, id)
            .await
            .map_err(gql_err)
    }

    /// Change how the caller syncs with a linked server; omitted values stay.
    async fn update_linked_server(
        &self,
        ctx: &Context<'_>,
        id: LinkedServerId,
        sync_enabled: Option<bool>,
        synced_until: Option<OffsetDateTime>,
    ) -> async_graphql::Result<LinkedServer> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let mut server = storage
            .linked_servers_for_user(principal.user_id)
            .await
            .map_err(gql_err)?
            .into_iter()
            .find(|server| server.id == id)
            .ok_or_else(|| async_graphql::Error::new("linked server not found"))?;
        if let Some(value) = sync_enabled {
            server.sync_enabled = value;
        }
        if let Some(value) = synced_until {
            server.synced_until = Some(value);
        }
        storage.upsert_linked_server(&server).await.map_err(gql_err)
    }

    /// Apply watch state from another server. An item only takes effect when it's
    /// newer than what's here, so replays are harmless and order doesn't matter.
    /// Unmatched titles and ones the caller can't access are skipped. Scrobblers
    /// aren't told: the server where it was watched already told them. Returns
    /// how many applied.
    async fn sync_watch_state(
        &self,
        ctx: &Context<'_>,
        items: Vec<WatchSyncItem>,
    ) -> async_graphql::Result<u32> {
        let user = access::request_user(ctx)?;
        let storage = storage(ctx)?;
        let mut applied = 0;
        for item in items {
            let Some(work) = sync_target(storage, &item).await? else {
                continue;
            };
            if !access::can_work(storage, user, work).await? {
                continue;
            }
            let target = PlayTarget::Work(work);
            let current = storage
                .user_data(user.user_id, target)
                .await
                .map_err(gql_err)?;
            if current.last_played.is_some_and(|at| at >= item.last_played) {
                continue;
            }
            storage
                .apply_watch_state(
                    user.user_id,
                    target,
                    item.played,
                    item.position_ms,
                    item.last_played,
                )
                .await
                .map_err(gql_err)?;
            applied += 1;
        }
        Ok(applied)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_origin;

    #[test]
    fn origins_are_reduced_to_what_a_browser_compares() {
        assert_eq!(
            normalize_origin(" HTTPS://Media.Example.com/ ").unwrap(),
            "https://media.example.com"
        );
        assert_eq!(
            normalize_origin("http://192.168.1.5:8096").unwrap(),
            "http://192.168.1.5:8096"
        );
        assert_eq!(
            normalize_origin("http://[::1]:8096").unwrap(),
            "http://[::1]:8096"
        );
    }

    #[test]
    fn anything_that_could_escape_a_csp_directive_is_refused() {
        for bad in [
            "media.example.com",
            "ftp://media.example.com",
            "https://",
            "https://example.com/path",
            "https://example.com; script-src *",
            "https://example.com evil.com",
            "https://example.com\r\nX-Evil: 1",
            "https://user:pw@example.com",
            "https://'unsafe-inline'",
            "https://*",
        ] {
            assert!(normalize_origin(bad).is_err(), "{bad}");
        }
    }
}
