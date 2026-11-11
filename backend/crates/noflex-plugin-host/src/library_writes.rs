// SPDX-License-Identifier: AGPL-3.0-only
//
// The imports through which a plugin writes library data it owns:
// `host-tags`, `host-collections` and `host-user-state`. Each is gated on
// its own manifest capability, audit-logged, and writes only data marked as
// the plugin's: tag assignments sourced `plugin:<id>`, collections keyed by
// (plugin, key), and user state tagged with the plugin as its source, which
// `user-history` then leaves out so a sync never reads its imports back.

use noflex_core::{DomainEvent, UserId, WorkId};
use noflex_storage::{
    StorageError,
    domain::{PluginCollectionSync, SourceTag, UserStateEntry, UserStateImport, Work},
    query::PageCursor,
};
use time::OffsetDateTime;
use wasmtime::component::{Accessor, HasSelf, Linker};

use crate::{
    PluginCtx,
    bindings::library_writes::noflex::plugin::{
        host_collections, host_tags, host_user_state, types,
    },
    dispatch::{current_principal, has_capability},
};

/// Most items one call carries: tags, collection members, history entries
/// or imports. Larger syncs chunk across `background` runs.
pub(crate) const MAX_BATCH: usize = 500;
const MAX_SLUG_BYTES: usize = 128;
const MAX_DISPLAY_BYTES: usize = 256;
const MAX_KEY_BYTES: usize = 128;
const MAX_TITLE_BYTES: usize = 256;
const MAX_OVERVIEW_BYTES: usize = 4096;

pub(crate) fn add_to_linker(linker: &mut Linker<PluginCtx>) -> wasmtime::Result<()> {
    host_tags::add_to_linker::<_, HasSelf<PluginCtx>>(linker, |ctx| ctx)?;
    host_collections::add_to_linker::<_, HasSelf<PluginCtx>>(linker, |ctx| ctx)?;
    host_user_state::add_to_linker::<_, HasSelf<PluginCtx>>(linker, |ctx| ctx)?;
    Ok(())
}

/// What every write import reads from the store before its first `.await`.
struct Call {
    plugin_id: String,
    capabilities: Vec<String>,
    tag_namespaces: Vec<String>,
    storage: Option<std::sync::Arc<dyn noflex_storage::Storage>>,
    events: Option<noflex_core::DomainEventBus>,
    principal: Option<UserId>,
}

impl Call {
    fn read(accessor: &Accessor<PluginCtx, HasSelf<PluginCtx>>) -> Self {
        accessor.with(|mut access| {
            let principal = current_principal(&mut access);
            let ctx = access.get();
            Self {
                plugin_id: ctx.plugin_id.0.clone(),
                capabilities: ctx.declared_capabilities.clone(),
                tag_namespaces: ctx.tag_namespaces.clone(),
                storage: ctx.storage.clone(),
                events: ctx.events.clone(),
                principal,
            }
        })
    }

    fn require(&self, capability: &str, import: &str) -> Result<(), types::PluginError> {
        if has_capability(&self.capabilities, capability) {
            Ok(())
        } else {
            Err(types::PluginError::CapabilityDenied(format!(
                "{import} requires the `{capability}` capability"
            )))
        }
    }

    fn storage(&self) -> Result<&dyn noflex_storage::Storage, types::PluginError> {
        self.storage
            .as_deref()
            .ok_or_else(|| types::PluginError::Unavailable("no library on this host".into()))
    }

    /// `user`, when it is the user this call acts for.
    fn bound_user(&self, user: &str) -> Result<UserId, types::PluginError> {
        let requested = user
            .parse::<uuid::Uuid>()
            .map(UserId::from_uuid)
            .map_err(|_| types::PluginError::BadInput(format!("invalid user id `{user}`")))?;
        match self.principal {
            None => Err(types::PluginError::Unavailable(
                "user state requires a user-bound invocation".into(),
            )),
            Some(principal) if principal != requested => Err(types::PluginError::CapabilityDenied(
                "requested user does not match the invocation principal".into(),
            )),
            Some(principal) => Ok(principal),
        }
    }

    fn emit(&self, event: DomainEvent) {
        if let Some(events) = &self.events {
            events.emit(event);
        }
    }
}

fn parse_work(id: &str) -> Result<WorkId, types::PluginError> {
    id.parse::<uuid::Uuid>()
        .map(WorkId::from_uuid)
        .map_err(|_| types::PluginError::BadInput(format!("invalid work id `{id}`")))
}

fn too_many(what: &str, count: usize) -> Option<types::PluginError> {
    (count > MAX_BATCH).then(|| {
        types::PluginError::BadInput(format!("{count} {what} in one call; at most {MAX_BATCH}"))
    })
}

fn internal(context: &str, error: StorageError) -> types::PluginError {
    types::PluginError::Internal(format!("{context}: {error}"))
}

// ----------------------------------------------------------------------
// `host-tags`
// ----------------------------------------------------------------------

impl host_tags::Host for PluginCtx {}

impl host_tags::HostWithStore<PluginCtx> for HasSelf<PluginCtx> {
    async fn set_tags(
        accessor: &Accessor<PluginCtx, Self>,
        work: String,
        namespace: String,
        tags: Vec<host_tags::TagDraft>,
    ) -> wasmtime::Result<Result<(), types::PluginError>> {
        let call = Call::read(accessor);
        let count = tags.len();
        let result = set_tags(&call, &work, &namespace, tags).await;
        crate::audit::library_write(
            &call.plugin_id,
            "set_tags",
            &namespace,
            count,
            result.is_ok(),
        );
        Ok(result)
    }
}

async fn set_tags(
    call: &Call,
    work: &str,
    namespace: &str,
    tags: Vec<host_tags::TagDraft>,
) -> Result<(), types::PluginError> {
    call.require("library.tags.write", "host-tags.set-tags")?;
    if !call
        .tag_namespaces
        .iter()
        .any(|claimed| claimed == namespace)
    {
        return Err(types::PluginError::CapabilityDenied(format!(
            "tag namespace `{namespace}` isn't claimed in the plugin manifest"
        )));
    }
    if let Some(error) = too_many("tags", tags.len()) {
        return Err(error);
    }
    let work = parse_work(work)?;
    let tags = tags
        .into_iter()
        .map(|tag| {
            let slug_ok = !tag.slug.is_empty()
                && tag.slug.len() <= MAX_SLUG_BYTES
                && tag
                    .slug
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            if !slug_ok {
                return Err(types::PluginError::BadInput(format!(
                    "tag slug `{}` must be lowercase letters, digits and `-`",
                    tag.slug
                )));
            }
            if tag.display.trim().is_empty() || tag.display.len() > MAX_DISPLAY_BYTES {
                return Err(types::PluginError::BadInput(format!(
                    "tag `{}` has an empty or overlong display",
                    tag.slug
                )));
            }
            if !(0.0..=1.0).contains(&tag.weight) {
                return Err(types::PluginError::BadInput(format!(
                    "tag `{}` has a weight outside [0, 1]",
                    tag.slug
                )));
            }
            Ok(SourceTag {
                slug: tag.slug,
                display: tag.display,
                weight: tag.weight,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let source = format!("plugin:{}", call.plugin_id);
    match call
        .storage()?
        .set_source_tags(work, namespace, &source, &tags)
        .await
    {
        Ok(()) => {}
        Err(StorageError::NotFound) => return Err(types::PluginError::NotFound),
        Err(e) => return Err(internal("set tags", e)),
    }
    call.emit(DomainEvent::PluginTagsSet {
        work,
        namespace: namespace.to_string(),
        plugin: call.plugin_id.clone(),
    });
    Ok(())
}

// ----------------------------------------------------------------------
// `host-collections`
// ----------------------------------------------------------------------

impl host_collections::Host for PluginCtx {}

impl host_collections::HostWithStore<PluginCtx> for HasSelf<PluginCtx> {
    async fn sync_collection(
        accessor: &Accessor<PluginCtx, Self>,
        key: String,
        title: String,
        overview: Option<String>,
        works: Vec<String>,
    ) -> wasmtime::Result<Result<u32, types::PluginError>> {
        let call = Call::read(accessor);
        let count = works.len();
        let result = sync_collection(&call, key.clone(), title, overview, works).await;
        crate::audit::library_write(
            &call.plugin_id,
            "sync_collection",
            &key,
            count,
            result.is_ok(),
        );
        Ok(result)
    }
}

async fn sync_collection(
    call: &Call,
    key: String,
    title: String,
    overview: Option<String>,
    works: Vec<String>,
) -> Result<u32, types::PluginError> {
    call.require(
        "library.collections.write",
        "host-collections.sync-collection",
    )?;
    if key.is_empty() || key.len() > MAX_KEY_BYTES {
        return Err(types::PluginError::BadInput(
            "collection key is empty or too long".into(),
        ));
    }
    if title.trim().is_empty() || title.len() > MAX_TITLE_BYTES {
        return Err(types::PluginError::BadInput(
            "collection title is empty or too long".into(),
        ));
    }
    if overview
        .as_ref()
        .is_some_and(|overview| overview.len() > MAX_OVERVIEW_BYTES)
    {
        return Err(types::PluginError::BadInput(
            "collection overview is too long".into(),
        ));
    }
    if let Some(error) = too_many("works", works.len()) {
        return Err(error);
    }
    let works = works
        .iter()
        .map(|id| parse_work(id))
        .collect::<Result<Vec<_>, _>>()?;
    let sync = PluginCollectionSync {
        plugin_id: call.plugin_id.clone(),
        key,
        title,
        overview: overview.filter(|overview| !overview.trim().is_empty()),
        works,
    };
    let collections = call
        .storage()?
        .sync_plugin_collection(&sync)
        .await
        .map_err(|e| internal("sync collection", e))?;
    call.emit(DomainEvent::PluginCollectionSynced {
        plugin: call.plugin_id.clone(),
        key: sync.key,
        collections: collections.iter().map(|c| c.id).collect(),
    });
    Ok(collections.len() as u32)
}

// ----------------------------------------------------------------------
// `host-user-state`
// ----------------------------------------------------------------------

impl host_user_state::Host for PluginCtx {}

impl host_user_state::HostWithStore<PluginCtx> for HasSelf<PluginCtx> {
    async fn user_history(
        accessor: &Accessor<PluginCtx, Self>,
        user: String,
        since: Option<i64>,
        cursor: Option<String>,
    ) -> wasmtime::Result<Result<host_user_state::HistoryPage, types::PluginError>> {
        let call = Call::read(accessor);
        let result = user_history(&call, &user, since, cursor).await;
        let count = result.as_ref().map_or(0, |page| page.entries.len());
        crate::audit::user_state(
            &call.plugin_id,
            "user_history",
            &user,
            count,
            result.is_ok(),
        );
        Ok(result)
    }

    async fn import_user_state(
        accessor: &Accessor<PluginCtx, Self>,
        user: String,
        items: Vec<host_user_state::StateImport>,
    ) -> wasmtime::Result<Result<u32, types::PluginError>> {
        let call = Call::read(accessor);
        let count = items.len();
        let result = import_user_state(&call, &user, items).await;
        crate::audit::user_state(
            &call.plugin_id,
            "import_user_state",
            &user,
            count,
            result.is_ok(),
        );
        Ok(result)
    }
}

fn from_millis(ms: i64) -> Result<OffsetDateTime, types::PluginError> {
    let at = OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .map_err(|_| types::PluginError::BadInput(format!("timestamp {ms} is out of range")))?;
    // A time ahead of now would win every later comparison.
    Ok(at.min(OffsetDateTime::now_utc()))
}

fn to_millis(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos() / 1_000_000).unwrap_or(i64::MAX)
}

async fn user_history(
    call: &Call,
    user: &str,
    since: Option<i64>,
    cursor: Option<String>,
) -> Result<host_user_state::HistoryPage, types::PluginError> {
    call.require("user-state.read", "host-user-state.user-history")?;
    let user = call.bound_user(user)?;
    let since = since.map(from_millis).transpose()?;
    let storage = call.storage()?;
    let page = match storage
        .user_state_history(user, since, MAX_BATCH as u32, cursor.map(PageCursor))
        .await
    {
        Ok(page) => page,
        Err(StorageError::InvalidInput(reason)) => {
            return Err(types::PluginError::BadInput(reason));
        }
        Err(e) => return Err(internal("read user history", e)),
    };
    let mut entries = Vec::with_capacity(page.items.len());
    for entry in page.items {
        let Some(seen) = Seen::of(&entry, &call.plugin_id) else {
            continue;
        };
        let work = match storage.get_work(entry.work).await {
            Ok(Some(work)) => work,
            Ok(None) => continue,
            Err(e) => return Err(internal("read work", e)),
        };
        entries.push(history_entry(storage, &work, seen).await);
    }
    Ok(host_user_state::HistoryPage {
        entries,
        next_cursor: page.next_cursor.map(|cursor| cursor.0),
    })
}

/// The parts of an entry a plugin may read: whatever it didn't import itself.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    played_at: Option<OffsetDateTime>,
    play_count: u32,
    rating: Option<(f32, OffsetDateTime)>,
    favorite: bool,
}

impl Seen {
    /// `None` when nothing is left once `plugin`'s own imports are hidden.
    fn of(entry: &UserStateEntry, plugin: &str) -> Option<Self> {
        let theirs = |source: &Option<String>| source.as_deref() == Some(plugin);
        let played_at = entry
            .played_at
            .filter(|_| entry.played && !theirs(&entry.played_source));
        let rating = entry
            .rating
            .zip(entry.rated_at)
            .filter(|_| !theirs(&entry.rating_source));
        let favorite = entry.favorite && !theirs(&entry.favorite_source);
        (played_at.is_some() || rating.is_some() || favorite).then_some(Self {
            played_at,
            play_count: if played_at.is_some() {
                entry.play_count
            } else {
                0
            },
            rating,
            favorite,
        })
    }
}

async fn history_entry(
    storage: &dyn noflex_storage::Storage,
    work: &Work,
    seen: Seen,
) -> host_user_state::HistoryEntry {
    let (series, season_number, episode_number) = crate::scrobbler::episode_place(storage, work)
        .await
        .unwrap_or_default();
    host_user_state::HistoryEntry {
        work: work.id.to_string(),
        kind: work_kind(work.kind),
        external_ids: external_ids(work),
        series_external_ids: series.as_ref().map(external_ids).unwrap_or_default(),
        season_number,
        episode_number,
        played_at: seen.played_at.map(to_millis),
        play_count: seen.play_count,
        rating: seen.rating.map(|(value, at)| host_user_state::UserRating {
            value,
            rated_at: to_millis(at),
        }),
        favorite: seen.favorite,
    }
}

async fn import_user_state(
    call: &Call,
    user: &str,
    items: Vec<host_user_state::StateImport>,
) -> Result<u32, types::PluginError> {
    call.require("user-state.write", "host-user-state.import-user-state")?;
    let user = call.bound_user(user)?;
    if let Some(error) = too_many("items", items.len()) {
        return Err(error);
    }
    let mut imports = Vec::with_capacity(items.len());
    for item in items {
        let rating = match item.rating {
            Some(rating) if rating.value.is_finite() && (0.0..=10.0).contains(&rating.value) => {
                Some((rating.value, from_millis(rating.rated_at)?))
            }
            Some(rating) => {
                return Err(types::PluginError::BadInput(format!(
                    "rating {} is outside 0–10",
                    rating.value
                )));
            }
            None => None,
        };
        imports.push(UserStateImport {
            work: parse_work(&item.work)?,
            played_at: item.played_at.map(from_millis).transpose()?,
            play_count: item.play_count,
            rating,
            favorite: item.favorite,
        });
    }
    let storage = call.storage()?;
    let ids = imports.iter().map(|item| item.work).collect::<Vec<_>>();
    let visible = storage
        .accessible_work_ids(user, &ids)
        .await
        .map_err(|e| internal("check access", e))?;
    imports.retain(|item| visible.contains(&item.work));
    let changed = storage
        .import_user_state(user, &call.plugin_id, &imports)
        .await
        .map_err(|e| internal("import user state", e))?;
    let count = changed.len() as u32;
    if !changed.is_empty() {
        call.emit(DomainEvent::UserStateImported {
            user,
            plugin: call.plugin_id.clone(),
            works: changed,
        });
    }
    Ok(count)
}

fn external_ids(work: &Work) -> Vec<types::ExternalId> {
    use noflex_core::model::ExternalIdKind as Core;
    use types::ExternalIdKind as Wit;
    work.external_ids
        .iter()
        .filter_map(|id| {
            let kind = match id.kind {
                Core::Tmdb => Wit::Tmdb,
                Core::Tvdb => Wit::Tvdb,
                Core::Imdb => Wit::Imdb,
                Core::MusicBrainzReleaseGroup => Wit::MusicBrainzReleaseGroup,
                Core::MusicBrainzArtist => Wit::MusicBrainzArtist,
                Core::MusicBrainzRecording => Wit::MusicBrainzRecording,
                Core::AniDb => Wit::AniDb,
                // The scanner's own keys mean nothing elsewhere.
                Core::Custom => return None,
            };
            Some(types::ExternalId {
                kind,
                value: id.value.clone(),
            })
        })
        .collect()
}

fn work_kind(kind: noflex_core::model::WorkKind) -> types::WorkKind {
    use noflex_core::model::WorkKind as Core;
    use types::WorkKind as Wit;
    match kind {
        Core::Movie => Wit::Movie,
        Core::Series => Wit::Series,
        Core::Season => Wit::Season,
        Core::Episode => Wit::Episode,
        Core::Album => Wit::Album,
        Core::Track => Wit::Track,
        Core::Artist => Wit::Artist,
        Core::AudioBook => Wit::AudioBook,
        Core::Photo => Wit::Photo,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> UserStateEntry {
        let at = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        UserStateEntry {
            work: WorkId::new(),
            played: true,
            played_at: Some(at),
            play_count: 2,
            played_source: None,
            rating: Some(7.0),
            rated_at: Some(at),
            rating_source: None,
            favorite: true,
            favorite_source: None,
        }
    }

    #[test]
    fn a_plugin_never_reads_back_what_it_imported() {
        let mine = UserStateEntry {
            played_source: Some("trakt".into()),
            rating_source: Some("trakt".into()),
            ..entry()
        };
        let seen = Seen::of(&mine, "trakt").expect("the favorite is still local");
        assert_eq!(seen.played_at, None);
        assert_eq!(seen.play_count, 0);
        assert_eq!(seen.rating, None);
        assert!(seen.favorite);

        let all_mine = UserStateEntry {
            favorite_source: Some("trakt".into()),
            ..mine.clone()
        };
        assert_eq!(Seen::of(&all_mine, "trakt"), None);
        assert!(
            Seen::of(&all_mine, "simkl").is_some(),
            "another plugin's imports are new to this one"
        );
    }

    #[test]
    fn an_untimed_rating_is_never_exported() {
        let untimed = UserStateEntry {
            rated_at: None,
            ..entry()
        };
        assert_eq!(Seen::of(&untimed, "trakt").expect("played").rating, None);
    }

    #[test]
    fn timestamps_are_clamped_to_now() {
        let future = to_millis(OffsetDateTime::now_utc()) + 86_400_000;
        assert!(from_millis(future).unwrap() <= OffsetDateTime::now_utc());
        assert_eq!(to_millis(from_millis(1_000).unwrap()), 1_000);
        assert!(from_millis(i64::MAX).is_err());
    }
}
