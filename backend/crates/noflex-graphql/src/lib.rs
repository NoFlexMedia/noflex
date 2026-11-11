// SPDX-License-Identifier: AGPL-3.0-only
//
// NoFlex's canonical GraphQL surface.
//
// Pattern lifted from assist's `assist-server/src/graphql/`:
//   - Query / Mutation / Subscription roots
//   - DataLoaders per relationship (added later)
//   - JWT auth extracted via Axum extension (added later)

// The merged Query and Mutation roots nest deeper than rustc's default query
// depth, each merged object adding a level.
#![recursion_limit = "256"]

use std::sync::Arc;

use async_graphql::{
    Context, Enum, MergedObject, Object, Schema, SimpleObject, Subscription,
    connection::Connection, dataloader::DataLoader,
};
use noflex_core::{DomainEventBus, EditionId, ExportJobId, LibraryId, UserId, WatchRoomId, WorkId};
use noflex_plugin_host::{HookKind, LifecycleBus, PluginRegistry};
use noflex_scanner::ScanRegistry;
use noflex_storage::{
    ItemPage, ListWorksQuery, Storage,
    domain::{
        ActivityEntry, ExportFormat, ExportJob, ExportJobStatus, HistoryRow, Library,
        LibraryConfig, LibraryKind, LibraryScanStatus, Work,
    },
    query::{ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX},
};
use time::OffsetDateTime;

mod metadata;
mod multi_server;
pub use metadata::MetadataSearch;
mod access;
pub mod admin_status;
mod auth;
mod browse;
mod collections;
mod connect;
mod credentials;
mod deletion;
mod devices;
mod embedding;
mod erasure;
mod external_auth;
mod live_tv;
mod paging;
mod playback;
mod playback_session;
mod plugin_links;
mod preferences;
mod profile;
mod recommend;
mod restrictions;
mod search;
mod sessions;
pub mod smart;
mod smart_marks;
mod stats;
pub mod subscriptions;
mod users;
mod watch;
pub use admin_status::{AdminStatusError, AdminStatusService};
pub use connect::ConnectHub;
pub use devices::SmartTranscodingDefaults;
pub use erasure::Erasure;
pub use external_auth::{ExternalAuthChallenge, ExternalAuthError, ExternalAuthService};
pub use live_tv::{LiveTvService, LiveTvServiceError};
pub use playback_session::{
    ActiveStream, ActiveStreamRecord, PlaybackRouteKind, PlaybackSession, PlaybackSessionService,
    PlaybackStart, SmartDownloadFile, StreamQuality, SubtitleDelivery, TranscodeProgress,
};
pub use plugin_links::{PluginLink, PluginLinkChallenge, PluginLinkService};
pub use subscriptions::{ExportEvent, ExportEventBroadcaster};
pub use watch::{WatchRoomAction, WatchRoomBus, WatchRoomEvent};

pub type NoflexSchema = Schema<Query, Mutation, Subscription>;

/// Per-operation complexity ceiling.
const MAX_COMPLEXITY: usize = 4000;
/// Cost of one credential-checking field: two fit in an operation.
const CREDENTIAL_COMPLEXITY: usize = MAX_COMPLEXITY / 2;

// Query / Mutation roots are `MergedObject`s so feature modules
// (`playback`, `browse`, …) can own their resolver blocks —
// async-graphql can't split one `#[Object] impl` across modules. The
// merged struct keeps the root type's GQL name (`Query`), so the SDL is
// identical to a single-block root. Kept as `//` comments, not doc
// comments: a doc comment would become the root type's SDL description.
#[derive(MergedObject, Default)]
pub struct Query(
    CoreQuery,
    admin_status::AdminStatusQuery,
    playback::PlaybackQuery,
    browse::BrowseQuery,
    devices::DevicesQuery,
    smart_marks::SmartMarksQuery,
    multi_server::MultiServerQuery,
    users::UsersQuery,
    auth::AuthQuery,
    preferences::PreferencesQuery,
    collections::CollectionsQuery,
    credentials::CredentialsQuery,
    external_auth::ExternalAuthQuery,
    plugin_links::PluginLinksQuery,
    sessions::SessionsQuery,
    search::SearchQuery,
    embedding::EmbeddingQuery,
    restrictions::RestrictionsQuery,
    watch::WatchQuery,
    connect::ConnectQuery,
    live_tv::LiveTvQuery,
    stats::StatsQuery,
);

#[derive(MergedObject, Default)]
pub struct Mutation(
    CoreMutation,
    playback::PlaybackMutation,
    devices::DevicesMutation,
    smart_marks::SmartMarksMutation,
    embedding::EmbeddingMutation,
    multi_server::MultiServerMutation,
    auth::AuthMutation,
    users::UsersMutation,
    profile::ProfileMutation,
    preferences::PreferencesMutation,
    collections::CollectionsMutation,
    credentials::CredentialsMutation,
    deletion::DeletionMutation,
    metadata::MetadataMutation,
    external_auth::ExternalAuthMutation,
    plugin_links::PluginLinksMutation,
    sessions::SessionsMutation,
    restrictions::RestrictionsMutation,
    watch::WatchMutation,
    connect::ConnectMutation,
    live_tv::LiveTvMutation,
);

pub struct Subscription;

#[derive(Default)]
struct CoreQuery;

#[derive(Default)]
struct CoreMutation;

#[derive(SimpleObject)]
struct ServerInfo {
    name: String,
    version: String,
    backend: String,
    is_configured: bool,
    /// Whether the server is erasing itself back to first-run setup, as its last
    /// administrator deleted their account. Everything but `serverInfo` is
    /// refused meanwhile, with code `SERVER_RESETTING`, and the server restarts
    /// on the way: ask until it answers with this false and `isConfigured`
    /// false.
    resetting: bool,
    /// Whether this server offers its media for keeping on a device. A client
    /// asks the server a title came from, as each decides for itself.
    downloads_allowed: bool,
    /// Whether the web client may load Google's Cast sender script. Other clients
    /// ignore it: they bring their own Cast support.
    cast_sdk: bool,
}

/// Whether the server's configuration admits Google's Cast sender script.
#[derive(Debug, Clone, Copy, Default)]
pub struct CastSdk(pub bool);

#[Object]
impl CoreQuery {
    async fn server_info(&self, ctx: &Context<'_>) -> async_graphql::Result<ServerInfo> {
        // The erasure holds storage, so this answers without it.
        if ctx
            .data_opt::<Arc<Erasure>>()
            .is_some_and(|erasure| erasure.is_resetting())
        {
            return Ok(ServerInfo {
                name: "noflex".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                backend: String::new(),
                is_configured: true,
                resetting: true,
                downloads_allowed: false,
                cast_sdk: false,
            });
        }
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let health = storage
            .health()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let users = storage
            .list_users()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(ServerInfo {
            name: "noflex".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            backend: health.backend,
            is_configured: !users.is_empty(),
            resetting: false,
            downloads_allowed: storage
                .downloads_allowed()
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?,
            cast_sdk: ctx.data_opt::<CastSdk>().is_some_and(|sdk| sdk.0),
        })
    }

    /// List all plugins currently loaded by the WASM plugin runtime.
    /// Returns an empty list if no PluginRegistry has been wired into the
    /// schema context (e.g. a build without the plugin runtime enabled).
    async fn plugins(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<noflex_plugin_host::Plugin>> {
        access::require_admin(ctx)?;
        let Ok(registry) = ctx.data::<Arc<PluginRegistry>>() else {
            return Ok(Vec::new());
        };
        Ok(registry.snapshot())
    }

    async fn libraries(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<Library>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let user = access::request_user(ctx)?;
        let mut libs = storage
            .list_libraries()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        if !user.is_admin {
            let grants = storage
                .list_user_libraries(user.user_id)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            libs.retain(|library| grants.contains(&library.id));
        }
        Ok(libs)
    }

    async fn library(
        &self,
        ctx: &Context<'_>,
        id: LibraryId,
    ) -> async_graphql::Result<Option<Library>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let user = access::request_user(ctx)?;
        let lib_id = id;
        if !user.is_admin
            && !storage
                .list_user_libraries(user.user_id)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?
                .contains(&lib_id)
        {
            return Ok(None);
        }
        storage
            .get_library(lib_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    async fn library_scan_status(
        &self,
        ctx: &Context<'_>,
        id: LibraryId,
    ) -> async_graphql::Result<LibraryScanStatus> {
        access::require_admin(ctx)?;
        let lib_id = id;
        let registry = ctx
            .data::<Arc<ScanRegistry>>()
            .map_err(|_| async_graphql::Error::new("scan registry not configured"))?;
        let status = registry.status(lib_id).await;
        Ok(scan_status_to_wire(status))
    }

    async fn search_metadata(
        &self,
        ctx: &Context<'_>,
        query: String,
        kind: metadata::WorkKind,
        year: Option<i32>,
    ) -> async_graphql::Result<Vec<metadata::MetadataMatch>> {
        access::require_admin(ctx)?;
        let search = ctx
            .data::<Arc<MetadataSearch>>()
            .map_err(|_| async_graphql::Error::new("metadata search not configured"))?;
        search.search_metadata(query, kind, year).await
    }

    /// How to credit each metadata provider this server has loaded, removed
    /// ones included, for anything that names a provider by id:
    /// `MetadataMatch.provider`, `Plugin.id`, `preferredProviders`. A title's
    /// own credits are on `Work.attributions`.
    async fn provider_attributions(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<noflex_storage::domain::ProviderAttribution>> {
        access::request_user(ctx)?;
        storage(ctx)?
            .list_provider_attributions()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))
    }

    /// Snapshot the active priority table for the admin UI. Returns
    /// the full set of (plugin, hook, work_kind) entries the
    /// dispatcher consults at request time.
    async fn plugin_overrides(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Vec<noflex_plugin_host::PluginOverrideEntry>> {
        access::require_admin(ctx)?;
        let table = ctx
            .data::<Arc<tokio::sync::RwLock<noflex_plugin_host::PriorityTable>>>()
            .map_err(|_| async_graphql::Error::new("priority table not configured"))?;
        let guard = table.read().await;
        Ok(guard.snapshot_gql())
    }

    /// The activity log, newest first. `kinds` filters by kind (empty
    /// = all); `since` is an inclusive floor on `occurredAt`, RFC 3339.
    /// Admins see every row; other users see only rows whose subject is
    /// a work or edition they can access, minus other users' playback,
    /// with `actor` nulled unless it is the caller. `first` defaults to
    /// 50, max 500. Requires authentication.
    #[graphql(complexity = "paging::cost(first, 50, 500, child_complexity)")]
    async fn activity(
        &self,
        ctx: &Context<'_>,
        kinds: Option<Vec<metadata::ActivityKind>>,
        since: Option<String>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, ActivityEntry>> {
        use time::format_description::well_known::Rfc3339;
        let window = paging::Window::new(first, after, 50, 500)?;
        let user = access::request_user(ctx)?;
        let storage = storage(ctx)?;
        let kind_filter: Vec<metadata::ActivityKind> = kinds.unwrap_or_default();
        let since_dt = since
            .as_deref()
            .map(|s| {
                OffsetDateTime::parse(s, &Rfc3339)
                    .map_err(|e| async_graphql::Error::new(format!("invalid since: {e}")))
            })
            .transpose()?;
        let fetch = |limit: u32, cursor: Option<noflex_storage::PageCursor>| {
            let kinds = &kind_filter;
            async move {
                storage
                    .list_activity(kinds, since_dt, limit, cursor)
                    .await
                    .map_err(gql_err)
            }
        };
        if user.is_admin {
            let resumed = window.after.is_some();
            return Ok(paging::from_item_page(
                fetch(window.first, window.after).await?,
                resumed,
            ));
        }
        paging::filtered(window, fetch, |rows| visible_activity(storage, user, rows)).await
    }

    /// "Continue Watching": unfinished works with a saved position, plus
    /// the next episode of each series the caller is following, most
    /// recent activity first. A series contributes one entry, and entries
    /// drop off after 90 days without activity; a newly added episode
    /// counts as activity. Ranked; `first` defaults to 20, max 100, at
    /// most 200 deep.
    #[graphql(complexity = "paging::cost(first, 20, 100, child_complexity)")]
    async fn continue_watching(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Work>> {
        let window = shelf_window(first, after)?;
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        let works = storage
            .continue_watching(
                user.user_id,
                OffsetDateTime::now_utc() - smart::CONTINUE_WATCHING_DECAY,
                overfetch(window),
            )
            .await
            .map_err(gql_err)?;
        Ok(window.connection(visible_works(storage, user, works, window.fetch()).await?))
    }

    /// "Recently Added": top-level works the caller can see, newest
    /// `addedAt` first — the default landing shelf of every Jellyfin
    /// client. `first` defaults to 20, max 100.
    #[graphql(complexity = "paging::cost(first, 20, 100, child_complexity)")]
    async fn recently_added(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Work>> {
        let window = paging::Window::new(first, after, 20, 100)?;
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        let resumed = window.after.is_some();
        let page = storage
            .list_works(&ListWorksQuery {
                access_user_id: user.access_user_id(),
                recursive: false,
                limit: window.first,
                cursor: window.after,
                ..Default::default()
            })
            .await
            .map_err(gql_err)?;
        Ok(paging::from_item_page(page, resumed))
    }

    /// Movies and series the caller hasn't started, most likely to enjoy
    /// first, judged by similarity to what they recently watched and how
    /// they responded to it. Callers with no history get the highest-rated
    /// titles. `category` narrows the list to one category, while everything
    /// they've watched still counts towards it. Ranked; `first` defaults to 20,
    /// max 100, at most 200 deep.
    #[graphql(complexity = "paging::cost(first, 20, 100, child_complexity)")]
    async fn recommendations(
        &self,
        ctx: &Context<'_>,
        category: Option<noflex_storage::domain::ContentCategory>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Work>> {
        let window = shelf_window(first, after)?;
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        let namespace = ctx
            .data::<Arc<PluginRegistry>>()
            .ok()
            .and_then(|registry| registry.embedder_claim())
            .map(|claim| claim.namespace);
        // Boxed because every resolver's future is laid out inside the merged
        // root's, and this one is large enough to overflow a worker's stack.
        let works = Box::pin(recommend::recommendations(
            storage.as_ref(),
            namespace.as_deref(),
            user,
            category,
            overfetch(window),
            OffsetDateTime::now_utc(),
        ))
        .await
        .map_err(gql_err)?;
        Ok(window.connection(visible_works(storage, user, works, window.fetch()).await?))
    }

    /// "Recently Played": the caller's played works, newest
    /// `lastPlayed` first. Ranked; `first` defaults to 20, max 100, at
    /// most 200 deep.
    #[graphql(complexity = "paging::cost(first, 20, 100, child_complexity)")]
    async fn recently_played(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Work>> {
        let window = shelf_window(first, after)?;
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        let works = storage
            .recently_played_for_user(user.user_id, overfetch(window))
            .await
            .map_err(gql_err)?;
        Ok(window.connection(visible_works(storage, user, works, window.fetch()).await?))
    }

    /// The caller's full play history, newest `lastPlayed` first; rows
    /// whose target the caller can no longer see are skipped. `first`
    /// defaults to 50, max 200.
    #[graphql(complexity = "paging::cost(first, 50, 200, child_complexity)")]
    async fn play_history(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, HistoryRow>> {
        let window = paging::Window::new(first, after, 50, 200)?;
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        let fetch = |limit: u32, cursor: Option<noflex_storage::PageCursor>| async move {
            let page = storage
                .list_play_history(user.user_id, cursor, limit)
                .await
                .map_err(gql_err)?;
            Ok(ItemPage {
                item_cursors: page.rows.iter().map(|row| row.cursor.clone()).collect(),
                end_cursor: page.rows.last().map(|row| row.cursor.clone()),
                has_next: page.next_cursor.is_some(),
                items: page.rows,
            })
        };
        paging::filtered(window, fetch, |rows| visible_history(storage, user, rows)).await
    }

    /// A single export job of the caller's; null when it was deleted,
    /// never existed, or belongs to someone else. The "Downloads" UI
    /// drives progress/completion off `status`.
    async fn export_job(
        &self,
        ctx: &Context<'_>,
        id: ExportJobId,
    ) -> async_graphql::Result<Option<ExportJob>> {
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        let job = storage.get_export_job(id).await.map_err(gql_err)?;
        Ok(job.filter(|job| job.user_id == user.user_id))
    }

    /// The caller's export jobs, newest `createdAt` first. The UI's
    /// "Downloads" view reads this on mount; the `exportJobStatus`
    /// subscription pushes status updates. `first` defaults to 50, max
    /// 100.
    #[graphql(
        complexity = "paging::cost(first, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX, child_complexity)"
    )]
    async fn export_jobs(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, ExportJob>> {
        let window = paging::Window::new(first, after, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX)?;
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        let resumed = window.after.is_some();
        let page = storage
            .list_export_jobs(user.user_id, window.first, window.after)
            .await
            .map_err(gql_err)?;
        Ok(paging::from_item_page(page, resumed))
    }
}

/// Home shelves are ranked lists capped at 200 deep.
fn shelf_window(
    first: Option<i32>,
    after: Option<String>,
) -> async_graphql::Result<paging::RankWindow> {
    paging::RankWindow::new(first, after, 20, 100, 200)
}

/// Storage rows to fetch for a ranked shelf: non-admins over-fetch
/// because access filtering drops some.
fn overfetch(window: paging::RankWindow) -> u32 {
    (window.fetch() as u32).saturating_mul(4).min(800)
}

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

/// Parse a bare `ID` into a typed id, for arguments whose kind travels
/// separately (`images(targetKind:, targetId:)`). Typed-id arguments
/// coerce themselves and never need this.
pub(crate) fn parse_id<T: From<uuid::Uuid>>(id: &str) -> async_graphql::Result<T> {
    uuid::Uuid::parse_str(id)
        .map(T::from)
        .map_err(|e| async_graphql::Error::new(format!("invalid ID {id:?}: {e}")))
}

async fn visible_works(
    storage: &Arc<dyn Storage>,
    user: noflex_storage::RequestUser,
    works: Vec<Work>,
    limit: usize,
) -> async_graphql::Result<Vec<Work>> {
    if user.is_admin {
        return Ok(works.into_iter().take(limit).collect());
    }
    let ids = works.iter().map(|work| work.id).collect::<Vec<_>>();
    let accessible = storage
        .accessible_work_ids(user.user_id, &ids)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    Ok(works
        .into_iter()
        .filter(|work| accessible.contains(&work.id))
        .take(limit)
        .collect())
}

/// History rows whose work or edition target the caller can still see.
async fn visible_history(
    storage: &Arc<dyn Storage>,
    user: noflex_storage::RequestUser,
    rows: Vec<(noflex_storage::PageCursor, HistoryRow)>,
) -> async_graphql::Result<Vec<(noflex_storage::PageCursor, HistoryRow)>> {
    let mut works = Vec::new();
    let mut editions = Vec::new();
    for (_, row) in &rows {
        match row.target_kind {
            noflex_core::PlayTargetKind::Work => works.push(WorkId::from_uuid(row.target_id)),
            noflex_core::PlayTargetKind::Edition => {
                editions.push(EditionId::from_uuid(row.target_id))
            }
        }
    }
    let (works, editions) = access::visible_targets(storage, user, &works, &editions).await?;
    Ok(rows
        .into_iter()
        .filter(|(_, row)| match row.target_kind {
            noflex_core::PlayTargetKind::Work => works.contains(&WorkId::from_uuid(row.target_id)),
            noflex_core::PlayTargetKind::Edition => {
                editions.contains(&EditionId::from_uuid(row.target_id))
            }
        })
        .collect())
}

/// Filter activity rows for a non-admin: the subject must parse as a
/// work or edition id the caller can access, playback rows must be the
/// caller's own, and other actors are redacted.
async fn visible_activity(
    storage: &Arc<dyn Storage>,
    user: noflex_storage::RequestUser,
    rows: Vec<(noflex_storage::PageCursor, ActivityEntry)>,
) -> async_graphql::Result<Vec<(noflex_storage::PageCursor, ActivityEntry)>> {
    use noflex_storage::domain::ActivityKind;

    let me = user.user_id.to_string();
    let subject_of = |row: &ActivityEntry| {
        row.subject
            .as_deref()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
    };
    let ids = rows
        .iter()
        .filter_map(|(_, row)| subject_of(row))
        .collect::<Vec<_>>();
    let (works, editions) = access::visible_targets(
        storage,
        user,
        &ids.iter()
            .copied()
            .map(WorkId::from_uuid)
            .collect::<Vec<_>>(),
        &ids.iter()
            .copied()
            .map(EditionId::from_uuid)
            .collect::<Vec<_>>(),
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(_, row)| {
            subject_of(row).is_some_and(|id| {
                works.contains(&WorkId::from_uuid(id))
                    || editions.contains(&EditionId::from_uuid(id))
            })
        })
        .filter_map(|(cursor, mut row)| {
            let own = row.actor.as_deref() == Some(me.as_str());
            let playback = matches!(
                row.kind,
                ActivityKind::PlayStarted
                    | ActivityKind::PlayProgress
                    | ActivityKind::PlayCompleted
            );
            if playback && !own {
                return None;
            }
            if !own {
                row.actor = None;
            }
            Some((cursor, row))
        })
        .collect())
}

async fn notify_library_changed(ctx: &Context<'_>, library: LibraryId) {
    if let Ok(watch) = ctx.data::<Arc<dyn LibraryWatchControl>>() {
        watch.library_changed(library).await;
    }
}

/// Keeps the server's filesystem watchers in step with library rows.
/// Implemented in `noflex-server`; behind a trait so the schema crate
/// never owns `notify` watchers.
#[async_trait::async_trait]
pub trait LibraryWatchControl: Send + Sync {
    /// `library` was created, changed, or deleted; reconcile its
    /// watcher with the stored row. Never fails the mutation: errors are
    /// the implementation's to log and surface in admin status.
    async fn library_changed(&self, library: LibraryId);
}

/// Everything a schema can be wired with, in one struct. Optional
/// pieces light up the resolvers/subscriptions that read them; a
/// resolver whose piece is absent errors at call time, so tests only
/// pay for the context they exercise.
#[derive(Clone, Default)]
pub struct SchemaContext {
    pub storage: Option<Arc<dyn Storage>>,
    /// Told about every library create/update/delete so filesystem
    /// watchers follow the library's roots and `watch_enabled`.
    pub library_watch: Option<Arc<dyn LibraryWatchControl>>,
    pub plugins: Option<Arc<PluginRegistry>>,
    pub scanner: Option<Arc<ScanRegistry>>,
    pub metadata: Option<Arc<MetadataSearch>>,
    pub priority_table: Option<Arc<tokio::sync::RwLock<noflex_plugin_host::PriorityTable>>>,
    /// Lives in the schema data (not an axum extension) so
    /// `completeSetupWizard` can issue a real JWT without separate
    /// HTTP-layer wiring.
    pub jwt: Option<Arc<noflex_auth::JwtConfig>>,
    /// Defaults to a fresh broadcaster when absent.
    pub export_events: Option<Arc<ExportEventBroadcaster>>,
    /// Lights up the `pluginLifecycle` subscription.
    pub lifecycle_bus: Option<LifecycleBus>,
    /// Domain-event fan-out for playback/scan mutations. Absent →
    /// producers stay silent (tests, minimal embeddings).
    pub event_bus: Option<DomainEventBus>,
    /// The stream runtime's session control plane. Absent →
    /// `startPlaybackSession`/`stopPlaybackSession` error with
    /// "playback sessions unavailable" (e.g. tests, or a build
    /// without the transcoder wired).
    pub playback_sessions: Option<Arc<dyn PlaybackSessionService>>,
    pub live_tv: Option<Arc<dyn LiveTvService>>,
    /// The per-user plugin link flow. Absent → `startPluginLink`
    /// errors with "plugin linking not configured".
    pub plugin_links: Option<Arc<dyn PluginLinkService>>,
    /// External login/link state machine and callback exchange.
    pub external_auth: Option<Arc<dyn ExternalAuthService>>,
    /// Mints user-bound tokens for trickplay and image URLs.
    pub media_url_signer: Option<Arc<dyn noflex_storage::MediaUrlSigner>>,
    /// In-process fan-out for persisted watch-room control events.
    pub watch_room_bus: Option<Arc<WatchRoomBus>>,
    /// Who is listening for commands, for NoFlex Connect. Defaults to a fresh
    /// hub when absent.
    pub connect_hub: Option<Arc<ConnectHub>>,
    /// Defaults to off.
    pub cast_sdk: Option<CastSdk>,
    /// Admin diagnostics (host, transcoder, indexing, logs). Absent →
    /// the admin status queries error with "admin status not configured".
    pub admin_status: Option<Arc<dyn AdminStatusService>>,
    /// Applies provider payloads and artwork for `refreshMetadata` /
    /// `identifyWork`. Absent → built per request from `plugins`,
    /// `storage` and `priority_table`, without artwork fetching.
    pub metadata_service: Option<Arc<noflex_plugin_host::MetadataService>>,
    /// Absent → `smartTranscoding` errors with "smart transcoding not
    /// configured".
    pub smart_transcoding: Option<devices::SmartTranscodingDefaults>,
    /// Woken when a viewer changes a smart transcode mark, so they don't wait
    /// for the next planning interval. Absent → the mark waits for it.
    pub smart_transcode_wake: Option<Arc<tokio::sync::Notify>>,
    /// The configured tone-mapping curve, until an administrator picks one.
    pub tonemap_curve: Option<noflex_core::TonemapCurve>,
    /// Told when a deleted account leaves something to purge, or the last
    /// administrator's erases the server. Defaults to a fresh one when absent,
    /// which nothing hears.
    pub erasure: Option<Arc<Erasure>>,
}

impl SchemaContext {
    /// Storage-only context; everything else stays dark.
    pub fn new(storage: Arc<dyn Storage>) -> Self {
        Self {
            storage: Some(storage),
            ..Self::default()
        }
    }
}

pub fn build_schema(ctx: SchemaContext) -> NoflexSchema {
    let mut builder = Schema::build(Query::default(), Mutation::default(), Subscription)
        .limit_depth(12)
        .limit_recursive_depth(32)
        .limit_complexity(MAX_COMPLEXITY)
        .limit_directives(100)
        .data(ctx.export_events.unwrap_or_default())
        .data(ctx.watch_room_bus.unwrap_or_default())
        .data(ctx.connect_hub.unwrap_or_default())
        .data(ctx.cast_sdk.unwrap_or_default())
        .data(Arc::new(noflex_auth::LoginThrottle::default()));
    let erasure = ctx.erasure.unwrap_or_default();
    builder = builder
        .data(erasure.clone())
        .extension(erasure::ResetGuard(erasure));
    if let Some(storage) = ctx.storage {
        builder = builder
            .data(DataLoader::new(
                noflex_storage::search::WorkLoader::new(storage.clone()),
                tokio::spawn,
            ))
            .data(DataLoader::new(
                noflex_storage::domain::EditionWorkLoader::new(storage.clone()),
                tokio::spawn,
            ))
            .data(DataLoader::new(
                noflex_storage::domain::LibraryLoader::new(storage.clone()),
                tokio::spawn,
            ))
            .data(storage);
    }
    if let Some(wake) = ctx.smart_transcode_wake {
        builder = builder.data(wake);
    }
    if let Some(plugins) = ctx.plugins {
        builder = builder.data(plugins);
    }
    if let Some(scanner) = ctx.scanner {
        builder = builder.data(scanner);
    }
    if let Some(metadata) = ctx.metadata {
        builder = builder.data(metadata);
    }
    if let Some(library_watch) = ctx.library_watch {
        builder = builder.data(library_watch);
    }
    if let Some(priority_table) = ctx.priority_table {
        builder = builder.data(priority_table);
    }
    if let Some(jwt) = ctx.jwt {
        builder = builder.data(jwt);
    }
    if let Some(lifecycle_bus) = ctx.lifecycle_bus {
        builder = builder.data(Arc::new(lifecycle_bus));
    }
    if let Some(event_bus) = ctx.event_bus {
        builder = builder.data(Arc::new(event_bus));
    }
    if let Some(playback_sessions) = ctx.playback_sessions {
        builder = builder.data(playback_sessions);
    }
    if let Some(live_tv) = ctx.live_tv {
        builder = builder.data(live_tv);
    }
    if let Some(plugin_links) = ctx.plugin_links {
        builder = builder.data(plugin_links);
    }
    if let Some(external_auth) = ctx.external_auth {
        builder = builder.data(external_auth);
    }
    if let Some(media_url_signer) = ctx.media_url_signer {
        builder = builder.data(media_url_signer);
    }
    if let Some(admin_status) = ctx.admin_status {
        builder = builder.data(admin_status);
    }
    if let Some(metadata_service) = ctx.metadata_service {
        builder = builder.data(metadata_service.indexing().clone());
        builder = builder.data(metadata_service);
    }
    if let Some(smart_transcoding) = ctx.smart_transcoding {
        builder = builder.data(smart_transcoding);
    }
    if let Some(tonemap_curve) = ctx.tonemap_curve {
        builder = builder.data(tonemap_curve);
    }
    builder.finish()
}

const ALREADY_CONFIGURED: &str = "server is already configured";

#[derive(SimpleObject)]
struct SetupResult {
    user_id: UserId,
    access_token: String,
    refresh_token: String,
}

#[Object]
impl CoreMutation {
    // Credential checks cost most of the per-operation complexity budget,
    // so aliasing can't batch hundreds of guesses into one request.
    #[graphql(complexity = "crate::CREDENTIAL_COMPLEXITY")]
    async fn complete_setup_wizard(
        &self,
        ctx: &Context<'_>,
        username: String,
        password: String,
    ) -> async_graphql::Result<SetupResult> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        // Checked up front so a misconfigured server never creates an
        // admin it can't mint a session for.
        ctx.data::<Arc<noflex_auth::JwtConfig>>()
            .map_err(|_| async_graphql::Error::new("auth not configured"))?;
        // Cheap early refusal so a configured server doesn't burn an
        // Argon2id run per anonymous call; the atomic insert below is
        // what actually decides the race.
        let configured = !storage
            .list_users()
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .is_empty();
        if configured {
            return Err(async_graphql::Error::new(ALREADY_CONFIGURED));
        }
        if username.is_empty() || password.is_empty() {
            return Err(async_graphql::Error::new(
                "username and password are required",
            ));
        }
        if username.len() > noflex_auth::MAX_USERNAME_BYTES
            || password.len() > noflex_auth::MAX_PASSWORD_BYTES
        {
            return Err(async_graphql::Error::new("credentials are too long"));
        }
        let hash = noflex_auth::hash_password(&password)
            .await
            .map_err(|_| async_graphql::Error::new("password processing failed"))?;
        let user = noflex_storage::domain::User {
            id: UserId::new(),
            username,
            display_name: None,
            password_hash: hash,
            is_admin: true,
            role: noflex_core::UserRole::Admin,
            created_at: OffsetDateTime::now_utc(),
            avatar_updated_at: None,
            has_pin: false,
        };
        let created = storage
            .create_initial_admin(&user)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        if !created {
            return Err(async_graphql::Error::new(ALREADY_CONFIGURED));
        }
        // Mint a session row so the wizard token is symmetric with a
        // `login` token — both carry a revocable `sid` claim.
        let (access_token, refresh_token) = auth::mint_session(ctx, &user, None, None).await?;
        Ok(SetupResult {
            user_id: user.id,
            access_token,
            refresh_token,
        })
    }

    async fn create_library(
        &self,
        ctx: &Context<'_>,
        name: String,
        kind: LibraryKind,
        paths: Vec<String>,
    ) -> async_graphql::Result<Library> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        access::require_admin(ctx)?;
        if name.trim().is_empty() {
            return Err(async_graphql::Error::new("name is required"));
        }
        if paths.is_empty() {
            return Err(async_graphql::Error::new("at least one path is required"));
        }
        let lib = Library {
            id: LibraryId::new(),
            name,
            kind,
            paths,
            config: LibraryConfig::default(),
        };
        storage
            .upsert_library(&lib)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        notify_library_changed(ctx, lib.id).await;
        Ok(lib)
    }

    async fn update_library(
        &self,
        ctx: &Context<'_>,
        id: LibraryId,
        name: Option<String>,
        kind: Option<LibraryKind>,
        paths: Option<Vec<String>>,
    ) -> async_graphql::Result<Library> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        access::require_admin(ctx)?;
        let lib_id = id;
        let mut current = storage
            .get_library(lib_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .ok_or_else(|| async_graphql::Error::new("library not found"))?;
        if let Some(n) = name {
            if n.trim().is_empty() {
                return Err(async_graphql::Error::new("name cannot be empty"));
            }
            current.name = n;
        }
        if let Some(k) = kind {
            current.kind = k;
        }
        if let Some(p) = paths {
            if p.is_empty() {
                return Err(async_graphql::Error::new("paths cannot be empty"));
            }
            current.paths = p;
        }
        storage
            .upsert_library(&current)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        notify_library_changed(ctx, current.id).await;
        Ok(current)
    }

    /// Delete a library and everything in it. Returns the deleted library.
    /// Admin only.
    async fn delete_library(
        &self,
        ctx: &Context<'_>,
        id: LibraryId,
    ) -> async_graphql::Result<Library> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        access::require_admin(ctx)?;
        let lib_id = id;
        let library = storage
            .get_library(lib_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .ok_or_else(|| async_graphql::Error::new("library not found"))?;
        storage
            .delete_library(lib_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        notify_library_changed(ctx, lib_id).await;
        Ok(library)
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_library_config(
        &self,
        ctx: &Context<'_>,
        id: LibraryId,
        preferred_languages: Option<Vec<String>>,
        preferred_providers: Option<Vec<String>>,
        excluded_folders: Option<Vec<String>>,
        watch_enabled: Option<bool>,
        scan_schedule: Option<noflex_storage::domain::ScanSchedule>,
        artwork_preference: Option<noflex_storage::domain::ArtworkPreference>,
        content_category: Option<noflex_storage::domain::LibraryCategory>,
    ) -> async_graphql::Result<Library> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        access::require_admin(ctx)?;
        let lib_id = id;
        let mut current = storage
            .get_library(lib_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .ok_or_else(|| async_graphql::Error::new("library not found"))?;
        if let Some(v) = preferred_languages {
            current.config.preferred_languages = v;
        }
        if let Some(v) = preferred_providers {
            current.config.preferred_providers = v;
        }
        if let Some(v) = excluded_folders {
            current.config.excluded_folders = v;
        }
        if let Some(v) = watch_enabled {
            current.config.watch_enabled = v;
        }
        if let Some(v) = scan_schedule {
            current.config.scan_schedule = v;
        }
        if let Some(v) = artwork_preference {
            current.config.artwork_preference = v;
        }
        if let Some(v) = content_category {
            current.config.content_category = v;
        }
        storage
            .upsert_library(&current)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        notify_library_changed(ctx, current.id).await;
        Ok(current)
    }

    /// Persist a per-(plugin, hook, work_kind) priority override
    /// to the storage. Returns the full active table after the write
    /// so the admin UI can re-render without a follow-up query.
    async fn set_plugin_override(
        &self,
        ctx: &Context<'_>,
        plugin_id: String,
        hook: metadata::HookKind,
        work_kind: metadata::WorkKindFilter,
        priority: i32,
    ) -> async_graphql::Result<Vec<noflex_plugin_host::PluginOverrideEntry>> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        access::require_admin(ctx)?;
        if priority < 0 {
            return Err(async_graphql::Error::new("priority must be >= 0"));
        }
        let registry = ctx
            .data::<Arc<PluginRegistry>>()
            .map_err(|_| async_graphql::Error::new("plugin registry not configured"))?;
        let plugin = registry
            .get(&noflex_plugin_host::PluginId(plugin_id.clone()))
            .ok_or_else(|| async_graphql::Error::new("plugin is not loaded"))?;
        if !plugin.hooks.contains(&hook) {
            return Err(async_graphql::Error::new(
                "plugin does not register the requested hook",
            ));
        }
        let hook_str = hook.as_str();
        let work_kind_str = work_kind.as_str();
        storage
            .upsert_plugin_override(&plugin_id, hook_str, work_kind_str, priority)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let table = ctx
            .data::<Arc<tokio::sync::RwLock<noflex_plugin_host::PriorityTable>>>()
            .map_err(|_| async_graphql::Error::new("priority table not configured"))?;
        let mut guard = table.write().await;
        guard
            .set(&plugin_id, hook_str, work_kind_str, priority)
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(guard.snapshot_gql())
    }

    /// Remove an admin priority override (persisted and live), reverting
    /// that (plugin, hook, work kind) to its manifest/config priority.
    /// Clearing an override that doesn't exist is a no-op. Admin only.
    async fn clear_plugin_override(
        &self,
        ctx: &Context<'_>,
        plugin_id: String,
        hook: metadata::HookKind,
        work_kind: metadata::WorkKindFilter,
    ) -> async_graphql::Result<Vec<noflex_plugin_host::PluginOverrideEntry>> {
        access::require_admin(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let table = ctx
            .data::<Arc<tokio::sync::RwLock<noflex_plugin_host::PriorityTable>>>()
            .map_err(|_| async_graphql::Error::new("priority table not configured"))?;
        let (hook_str, work_kind_str) = (hook.as_str(), work_kind.as_str());
        storage
            .delete_plugin_override(&plugin_id, hook_str, work_kind_str)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let mut guard = table.write().await;
        guard
            .clear(&plugin_id, hook_str, work_kind_str)
            .map_err(async_graphql::Error::new)?;
        Ok(guard.snapshot_gql())
    }

    async fn trigger_library_scan(
        &self,
        ctx: &Context<'_>,
        id: LibraryId,
    ) -> async_graphql::Result<LibraryScanStatus> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        access::require_admin(ctx)?;
        let registry = ctx
            .data::<Arc<ScanRegistry>>()
            .map_err(|_| async_graphql::Error::new("scan registry not configured"))?;
        let lib_id = id;
        let lib = storage
            .get_library(lib_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .ok_or_else(|| async_graphql::Error::new("library not found"))?;
        let status = registry.trigger(lib.id).await;
        Ok(scan_status_to_wire(status))
    }

    /// Enqueue a new export job. The worker
    /// (`DownloadWorker` in `noflex-server`) picks it up on its
    /// next poll, runs ffmpeg, and stamps the row through
    /// `Completed` / `Failed` / `Cancelled`. The mutation
    /// returns the freshly-created `ExportJob` with `status =
    /// Queued` so the caller can `subscribe` to its progress.
    async fn create_export_job(
        &self,
        ctx: &Context<'_>,
        edition_id: EditionId,
        format: ExportFormat,
    ) -> async_graphql::Result<ExportJob> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let user = access::request_user(ctx)?;
        let ed_id = edition_id;
        if !access::can_edition(storage, user, ed_id).await? {
            return Err(async_graphql::Error::new("edition not found"));
        }
        devices::require_downloads_allowed(storage).await?;
        // Asking twice for what's already there or on its way is one request.
        if let Some(existing) = storage
            .user_export_job(user.user_id, ed_id, format)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
        {
            return Ok(existing);
        }
        let job = ExportJob {
            id: noflex_core::ExportJobId::new(),
            user_id: user.user_id,
            edition_id: ed_id,
            format,
            status: ExportJobStatus::Queued,
            progress: 0.0,
            output_path: None,
            error: None,
            created_at: OffsetDateTime::now_utc(),
            started_at: None,
            finished_at: None,
        };
        storage
            .create_export_job(&job)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(job)
    }

    async fn cancel_export_job(
        &self,
        ctx: &Context<'_>,
        id: ExportJobId,
    ) -> async_graphql::Result<ExportJob> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let user = access::request_user(ctx)?;
        let job_id = id;
        let cancellation = storage
            .request_export_job_cancellation(job_id, user.user_id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .ok_or_else(|| async_graphql::Error::new("export job not found"))?;
        if !cancellation.changed {
            return Err(async_graphql::Error::new("export job cannot be cancelled"));
        }
        Ok(cancellation.job)
    }

    /// Say the caller no longer needs this copy. The file goes once nobody else's
    /// job shares it. One still queued or running has to be cancelled first.
    /// Returns the removed job.
    async fn delete_export_job(
        &self,
        ctx: &Context<'_>,
        id: ExportJobId,
    ) -> async_graphql::Result<ExportJob> {
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let user = access::request_user(ctx)?;
        let job = storage
            .get_export_job(id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .filter(|job| job.user_id == user.user_id)
            .ok_or_else(|| async_graphql::Error::new("export job not found"))?;
        if matches!(
            job.status,
            ExportJobStatus::Queued | ExportJobStatus::Running | ExportJobStatus::CancelRequested
        ) {
            return Err(async_graphql::Error::new(
                "cancel this export before removing it",
            ));
        }
        storage
            .delete_export_job(id)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        if let Some(output) = job.output_path.as_deref()
            && !storage
                .export_output_in_use(output)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?
            && let Err(error) = tokio::fs::remove_file(output).await
        {
            tracing::warn!(%error, output, "unused export not removed");
        }
        Ok(job)
    }
}

/// Per-job status updates streamed to the `Downloads` UI. The
/// `DownloadWorker` calls
/// `ExportEventBroadcaster::emit_status` on every state
/// transition; this subscription filters the broadcast
/// channel by job id and yields the matching events.
///
/// Filtering happens in the subscription body itself
/// (rather than in the worker) so the worker can be
/// fire-and-forget: a slow client doesn't back-pressure the
/// queue, and an event for a different job never even
/// crosses the wire.
#[derive(SimpleObject)]
pub struct ExportStatusEvent {
    pub id: ExportJobId,
    pub status: ExportJobStatus,
    pub progress: f32,
}

/// Tagged kind for a [`PluginLifecycleEvent`]. Mirrors
/// the 5-variant canonical `PluginLifecycleEvent` enum; the GQL
/// client uses this as the discriminator when rendering the
/// lifecycle log.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum PluginLifecycleKind {
    Discovered,
    Loaded,
    LoadFailed,
    Unloaded,
    Trapped,
}

/// A single lifecycle event the GQL client receives.
/// `kind` is the discriminator; the other fields are filled in
/// per-variant. Any field not applicable to the variant is `None`
/// (or empty for the `hooks` list).
///
/// The GQL type shares the name of the canonical Rust enum
/// (`noflex_plugin_host::PluginLifecycleEvent`) by design: the
/// canonical enum is a Rust tagged union whose variants have
/// different shapes, so `async_graphql::SimpleObject` can't
/// derive on it directly. The flat `struct` form here is the
/// GQL wire shape; the `From` impl below is a wire-format
/// conversion (enum → discriminated struct), not a mirror.
#[derive(SimpleObject)]
pub struct PluginLifecycleEvent {
    pub kind: PluginLifecycleKind,
    pub plugin_id: Option<String>,
    pub path: Option<String>,
    pub sha256: Option<String>,
    pub hooks: Vec<HookKind>,
    pub reason: Option<String>,
    pub trap: Option<String>,
}

impl From<noflex_plugin_host::PluginLifecycleEvent> for PluginLifecycleEvent {
    fn from(ev: noflex_plugin_host::PluginLifecycleEvent) -> Self {
        match ev {
            noflex_plugin_host::PluginLifecycleEvent::Discovered(d) => {
                let hooks = d
                    .manifest
                    .hooks
                    .iter()
                    .filter_map(|s| HookKind::parse(s))
                    .collect();
                Self {
                    kind: PluginLifecycleKind::Discovered,
                    plugin_id: Some(d.manifest.plugin_id),
                    path: Some(d.path.display().to_string()),
                    sha256: Some(d.binary_sha256),
                    hooks,
                    reason: None,
                    trap: None,
                }
            }
            noflex_plugin_host::PluginLifecycleEvent::Loaded {
                plugin_id,
                binary_sha256,
                hooks,
            } => Self {
                kind: PluginLifecycleKind::Loaded,
                plugin_id: Some(plugin_id.0),
                path: None,
                sha256: Some(binary_sha256),
                hooks,
                reason: None,
                trap: None,
            },
            noflex_plugin_host::PluginLifecycleEvent::LoadFailed { path, reason } => Self {
                kind: PluginLifecycleKind::LoadFailed,
                plugin_id: None,
                path: Some(path.display().to_string()),
                sha256: None,
                hooks: Vec::new(),
                reason: Some(reason),
                trap: None,
            },
            noflex_plugin_host::PluginLifecycleEvent::Unloaded { plugin_id, reason } => Self {
                kind: PluginLifecycleKind::Unloaded,
                plugin_id: Some(plugin_id.0),
                path: None,
                sha256: None,
                hooks: Vec::new(),
                reason: Some(format!("{reason:?}")),
                trap: None,
            },
            noflex_plugin_host::PluginLifecycleEvent::Trapped {
                plugin_id,
                hook,
                trap,
            } => Self {
                kind: PluginLifecycleKind::Trapped,
                plugin_id: Some(plugin_id.0),
                path: None,
                sha256: None,
                hooks: vec![hook],
                reason: None,
                trap: Some(trap),
            },
        }
    }
}

#[Subscription]
impl Subscription {
    async fn export_job_status(
        &self,
        ctx: &Context<'_>,
        id: ExportJobId,
    ) -> async_graphql::Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = ExportStatusEvent> + Send + 'static>>,
    > {
        let user = access::request_user(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let target = id;
        let owned = storage
            .get_export_job(target)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .is_some_and(|job| job.user_id == user.user_id);
        if !owned {
            return Err(async_graphql::Error::new("export job not found"));
        }
        let broadcaster = ctx
            .data::<Arc<ExportEventBroadcaster>>()
            .map_err(|_| async_graphql::Error::new("export event broadcaster not configured"))?
            .clone();
        let mut rx = broadcaster.subscribe();
        Ok(Box::pin(async_stream::stream! {
            loop {
                match rx.recv().await {
                    Ok(ExportEvent::StatusChanged { id: ev_id, status, progress, .. })
                        if ev_id == target =>
                    {
                        yield ExportStatusEvent {
                            id: ev_id,
                            status,
                            progress,
                        };
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }))
    }

    async fn watch_room_events(
        &self,
        ctx: &Context<'_>,
        room_id: WatchRoomId,
    ) -> async_graphql::Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = WatchRoomEvent> + Send + 'static>>,
    > {
        let principal = access::request_user(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let id = room_id;
        // Subscribe before the membership check so no event slips
        // between the check and the first `recv`.
        let mut rx = ctx
            .data::<Arc<WatchRoomBus>>()
            .map_err(|_| async_graphql::Error::new("watch room bus not configured"))?
            .subscribe();
        let user = principal.user_id;
        if !watch::is_member(storage, id, user).await? {
            return Err(async_graphql::Error::new("watch room not found"));
        }
        let storage = storage.clone();
        // Membership is re-checked before every delivery, so a member who
        // left (or was dropped when the room ended) stops receiving
        // events; the stream closes after the room's terminal event.
        Ok(Box::pin(async_stream::stream! {
            loop {
                match rx.recv().await {
                    Ok(event) if event.room.id == id => {
                        let ended = event.action == WatchRoomAction::Stop
                            || event.room.state == noflex_storage::domain::WatchRoomState::Ended;
                        if !ended && !watch::is_member(&storage, id, user).await.unwrap_or(false) {
                            break;
                        }
                        yield event;
                        if ended {
                            break;
                        }
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }))
    }

    /// Commands for one of the caller's devices, for as long as it listens.
    /// A device listens in one place: subscribing again replaces the first
    /// subscription. While one is open, and the device keeps calling
    /// `reportDeviceState`, `connectDevices` shows it online.
    async fn device_commands(
        &self,
        ctx: &Context<'_>,
        device_id: noflex_core::DeviceId,
    ) -> async_graphql::Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = connect::DeviceCommand> + Send + 'static>>,
    > {
        connect::device_commands(ctx, device_id).await
    }

    /// The caller's devices now, and again whenever one comes, goes or says
    /// what it's playing.
    async fn connect_devices(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = Vec<connect::ConnectDevice>> + Send + 'static>,
        >,
    > {
        connect::watch_devices(ctx).await
    }

    /// Stream every plugin lifecycle event the host emits.
    /// Subscribers receive a flat, tagged event with
    /// `kind` as the discriminator; per-variant fields
    /// (`path`, `reason`, `trap`, …) are `None` when not
    /// applicable. The stream only terminates when the
    /// bus is closed (server shutdown).
    async fn plugin_lifecycle(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = PluginLifecycleEvent> + Send + 'static>>,
    > {
        access::require_admin(ctx)?;
        let bus = ctx
            .data::<Arc<LifecycleBus>>()
            .map_err(|_| async_graphql::Error::new("lifecycle bus not configured"))?
            .clone();
        let mut rx = bus.subscribe();
        Ok(Box::pin(async_stream::stream! {
            loop {
                match rx.recv().await {
                    Ok(ev) => yield PluginLifecycleEvent::from(ev),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }))
    }

    /// Live host/process samples for admin gauges: the latest sample
    /// on subscribe, then one per sampler tick. Administrator only.
    async fn system_status_updates(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<
        std::pin::Pin<Box<dyn futures::Stream<Item = Arc<admin_status::SystemStatus>> + Send>>,
    > {
        admin_status::system_status_stream(ctx)
    }
}

fn scan_status_to_wire(s: noflex_scanner::ScanStatus) -> LibraryScanStatus {
    use time::format_description::well_known::Rfc3339;
    let format_rfc3339 =
        |dt: time::OffsetDateTime| dt.format(&Rfc3339).unwrap_or_else(|_| dt.to_string());
    let summary = |sum: noflex_scanner::ScanSummary| noflex_storage::domain::ScanSummary {
        added: sum.added as i32,
        updated: sum.updated as i32,
        removed: sum.removed as i32,
        errored: sum.errored as i32,
    };
    match s {
        noflex_scanner::ScanStatus::Idle => LibraryScanStatus {
            state: noflex_storage::domain::ScanState::Idle,
            started_at: None,
            finished_at: None,
            summary: None,
            error: None,
        },
        noflex_scanner::ScanStatus::Running { started_at } => LibraryScanStatus {
            state: noflex_storage::domain::ScanState::Running,
            started_at: Some(format_rfc3339(started_at)),
            finished_at: None,
            summary: None,
            error: None,
        },
        noflex_scanner::ScanStatus::Completed {
            started_at,
            finished_at,
            summary: sum,
        } => LibraryScanStatus {
            state: noflex_storage::domain::ScanState::Completed,
            started_at: Some(format_rfc3339(started_at)),
            finished_at: Some(format_rfc3339(finished_at)),
            summary: Some(summary(sum)),
            error: None,
        },
        noflex_scanner::ScanStatus::Failed {
            started_at,
            finished_at,
            error,
        } => LibraryScanStatus {
            state: noflex_storage::domain::ScanState::Failed,
            started_at: Some(format_rfc3339(started_at)),
            finished_at: Some(format_rfc3339(finished_at)),
            summary: None,
            error: Some(error),
        },
    }
}
