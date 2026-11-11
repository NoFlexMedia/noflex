// SPDX-License-Identifier: AGPL-3.0-only
//
// Collections are admin-curated, library-scoped groupings; reads are
// ungated like the rest of the browse surface, while writes are admin-only.
// Playlists are user-owned; every playlist resolver is gated on
// owner-or-admin, and non-owners get the same "not found" answer as for a
// missing row so playlist existence doesn't leak across users.

use std::sync::Arc;

use async_graphql::{Context, ErrorExtensions as _, Object, connection::Connection};
use noflex_core::{CollectionId, LibraryId, PlayTarget, PlaylistId, WorkId};
use noflex_storage::{
    Storage, StorageError,
    domain::{Collection, Playlist},
    query::{ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX},
};
use time::OffsetDateTime;

use crate::{access, paging, playback::PlayTargetInput, users::authed_principal};

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

/// An edit to a collection a plugin syncs, which would be overwritten at its
/// next sync: detach it first.
pub const PLUGIN_OWNED_COLLECTION: &str = "PLUGIN_OWNED_COLLECTION";

fn refuse_plugin_owned(collection: &Collection) -> async_graphql::Result<()> {
    match &collection.plugin_id {
        Some(plugin) => Err(async_graphql::Error::new(format!(
            "this collection is synced by the {plugin} plugin; detach it to edit it"
        ))
        .extend_with(|_, ext| ext.set("code", PLUGIN_OWNED_COLLECTION))),
        None => Ok(()),
    }
}

/// The playlist the caller is allowed to touch: it must exist, and the
/// caller must own it or be an admin. Both failure modes report "not
/// found" — a non-owner learns nothing about the row.
async fn authed_playlist(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
    id: PlaylistId,
) -> async_graphql::Result<Playlist> {
    let principal = authed_principal(ctx)?;
    let playlist = storage
        .get_playlist(id)
        .await
        .map_err(gql_err)?
        .ok_or_else(|| async_graphql::Error::new("playlist not found"))?;
    if !principal.is_admin && playlist.owner_id != principal.user_id {
        return Err(async_graphql::Error::new("playlist not found"));
    }
    Ok(playlist)
}

/// Most ids one membership write may carry.
pub(crate) const MAX_ITEMS_PER_WRITE: usize = 1000;

pub(crate) fn check_item_count(len: usize) -> async_graphql::Result<()> {
    if len > MAX_ITEMS_PER_WRITE {
        return Err(async_graphql::Error::new(format!(
            "at most {MAX_ITEMS_PER_WRITE} items per request"
        )));
    }
    Ok(())
}

/// Every input must resolve to a work/edition the caller can see before
/// the write lands; a dead id in the middle of a list fails the whole
/// mutation, naming the first id that didn't resolve. Access is checked
/// in one batch, not per item.
async fn resolve_targets(
    ctx: &Context<'_>,
    storage: &Arc<dyn Storage>,
    inputs: &[PlayTargetInput],
) -> async_graphql::Result<Vec<PlayTarget>> {
    check_item_count(inputs.len())?;
    let user = access::request_user(ctx)?;
    let targets = inputs
        .iter()
        .map(PlayTargetInput::to_play_target)
        .collect::<async_graphql::Result<Vec<_>>>()?;
    let mut works = Vec::new();
    let mut editions = Vec::new();
    for target in &targets {
        match *target {
            PlayTarget::Work(id) => works.push(id),
            PlayTarget::Edition(id) => editions.push(id),
        }
    }
    let (visible_works, visible_editions) =
        access::visible_targets(storage, user, &works, &editions).await?;
    for (input, target) in inputs.iter().zip(&targets) {
        let (visible, kind) = match *target {
            PlayTarget::Work(id) => (visible_works.contains(&id), "work"),
            PlayTarget::Edition(id) => (visible_editions.contains(&id), "edition"),
        };
        if !visible {
            return Err(async_graphql::Error::new(format!(
                "{kind} not found: {}",
                input.id.0
            )));
        }
    }
    Ok(targets)
}

#[derive(Default)]
pub struct CollectionsQuery;

#[Object]
impl CollectionsQuery {
    /// Collections in a library, ordered by sort title. `first`
    /// defaults to 50, max 100.
    #[graphql(
        complexity = "paging::cost(first, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX, child_complexity)"
    )]
    async fn collections(
        &self,
        ctx: &Context<'_>,
        library_id: LibraryId,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Collection>> {
        let window = paging::Window::new(first, after, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX)?;
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        if !user.is_admin
            && !storage
                .list_user_libraries(user.user_id)
                .await
                .map_err(gql_err)?
                .contains(&library_id)
        {
            return Ok(paging::empty());
        }
        let resumed = window.after.is_some();
        let page = storage
            .list_collections(library_id, window.first, window.after)
            .await
            .map_err(gql_err)?;
        Ok(paging::from_item_page(page, resumed))
    }

    /// A single collection by id; null when it doesn't exist.
    async fn collection(
        &self,
        ctx: &Context<'_>,
        id: CollectionId,
    ) -> async_graphql::Result<Option<Collection>> {
        let storage = storage(ctx)?;
        let user = access::request_user(ctx)?;
        let collection = storage.get_collection(id).await.map_err(gql_err)?;
        let Some(collection) = collection else {
            return Ok(None);
        };
        if !user.is_admin
            && !storage
                .list_user_libraries(user.user_id)
                .await
                .map_err(gql_err)?
                .contains(&collection.library_id)
        {
            return Ok(None);
        }
        Ok(Some(collection))
    }

    /// The calling user's playlists, ordered by title. `first`
    /// defaults to 50, max 100.
    #[graphql(
        complexity = "paging::cost(first, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX, child_complexity)"
    )]
    async fn playlists(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, Playlist>> {
        let window = paging::Window::new(first, after, ITEM_PAGE_DEFAULT, ITEM_PAGE_MAX)?;
        let principal = authed_principal(ctx)?;
        let resumed = window.after.is_some();
        let page = storage(ctx)?
            .list_playlists(principal.user_id, window.first, window.after)
            .await
            .map_err(gql_err)?;
        Ok(paging::from_item_page(page, resumed))
    }

    /// A single playlist by id. Null when it doesn't exist or belongs
    /// to someone else (admins see all).
    async fn playlist(
        &self,
        ctx: &Context<'_>,
        id: PlaylistId,
    ) -> async_graphql::Result<Option<Playlist>> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let playlist = storage.get_playlist(id).await.map_err(gql_err)?;
        Ok(playlist.filter(|p| principal.is_admin || p.owner_id == principal.user_id))
    }
}

#[derive(Default)]
pub struct CollectionsMutation;

#[Object]
impl CollectionsMutation {
    /// Create a collection in a library. `sortTitle` defaults to
    /// `title`. Admin only.
    async fn create_collection(
        &self,
        ctx: &Context<'_>,
        library_id: LibraryId,
        title: String,
        sort_title: Option<String>,
        overview: Option<String>,
    ) -> async_graphql::Result<Collection> {
        access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        if title.trim().is_empty() {
            return Err(async_graphql::Error::new("title is required"));
        }
        let lib_id = library_id;
        storage
            .get_library(lib_id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("library not found"))?;
        let now = OffsetDateTime::now_utc();
        let collection = Collection {
            id: CollectionId::new(),
            library_id: lib_id,
            sort_title: sort_title.unwrap_or_else(|| title.clone()),
            title,
            overview,
            created_at: now,
            updated_at: now,
            plugin_id: None,
            plugin_key: None,
        };
        storage
            .create_collection(&collection)
            .await
            .map_err(gql_err)?;
        Ok(collection)
    }

    /// Rename / re-annotate a collection. Unpassed fields keep their
    /// current values. Admin only.
    async fn update_collection(
        &self,
        ctx: &Context<'_>,
        id: CollectionId,
        title: Option<String>,
        sort_title: Option<String>,
        overview: Option<String>,
    ) -> async_graphql::Result<Collection> {
        access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let mut collection = storage
            .get_collection(id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("collection not found"))?;
        refuse_plugin_owned(&collection)?;
        if let Some(title) = title {
            if title.trim().is_empty() {
                return Err(async_graphql::Error::new("title cannot be empty"));
            }
            collection.title = title;
        }
        if let Some(sort_title) = sort_title {
            collection.sort_title = sort_title;
        }
        if let Some(overview) = overview {
            collection.overview = Some(overview);
        }
        collection.updated_at = OffsetDateTime::now_utc();
        storage
            .update_collection(&collection)
            .await
            .map_err(gql_err)?;
        Ok(collection)
    }

    /// Make a collection a plugin syncs an ordinary one an administrator
    /// curates. The plugin never writes its list to this library again, and
    /// what it holds now stays. Admin only.
    async fn detach_collection(
        &self,
        ctx: &Context<'_>,
        id: CollectionId,
    ) -> async_graphql::Result<Collection> {
        let admin = access::require_admin(ctx)?;
        match storage(ctx)?.detach_collection(id).await {
            Ok(collection) => {
                tracing::info!(collection = %id, admin = %admin.user_id, "collection detached from its plugin");
                Ok(collection)
            }
            Err(StorageError::NotFound) => Err(async_graphql::Error::new("collection not found")),
            Err(StorageError::InvalidInput(reason)) => Err(async_graphql::Error::new(reason)),
            Err(e) => Err(gql_err(e)),
        }
    }

    /// Delete a collection and its membership list. Returns it, or null
    /// when the id didn't exist. A collection a plugin syncs comes back at
    /// its next sync unless it's detached first. Admin only.
    async fn delete_collection(
        &self,
        ctx: &Context<'_>,
        id: CollectionId,
    ) -> async_graphql::Result<Option<Collection>> {
        access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let Some(collection) = storage.get_collection(id).await.map_err(gql_err)? else {
            return Ok(None);
        };
        match storage.delete_collection(id).await {
            Ok(()) => Ok(Some(collection)),
            Err(StorageError::NotFound) => Ok(None),
            Err(e) => Err(gql_err(e)),
        }
    }

    /// Replace a collection's membership wholesale. Every id must
    /// resolve and belong to the collection's library; the error
    /// names the first id that doesn't. Admin only.
    async fn set_collection_items(
        &self,
        ctx: &Context<'_>,
        id: CollectionId,
        work_ids: Vec<WorkId>,
    ) -> async_graphql::Result<Collection> {
        access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let collection = storage
            .get_collection(id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("collection not found"))?;
        refuse_plugin_owned(&collection)?;
        check_item_count(work_ids.len())?;
        let ids = work_ids;
        let hydrated = storage.get_works(&ids).await.map_err(gql_err)?;
        let by_id = hydrated
            .into_iter()
            .map(|work| (work.id, work))
            .collect::<std::collections::HashMap<_, _>>();
        for id in &ids {
            let work = by_id
                .get(id)
                .ok_or_else(|| async_graphql::Error::new(format!("work not found: {id}")))?;
            if work.library_id != collection.library_id {
                return Err(async_graphql::Error::new(format!(
                    "work {id} does not belong to this collection's library"
                )));
            }
        }
        storage
            .set_collection_items(collection.id, &ids)
            .await
            .map_err(gql_err)?;
        Ok(collection)
    }

    /// Create a playlist owned by the caller.
    async fn create_playlist(
        &self,
        ctx: &Context<'_>,
        title: String,
        overview: Option<String>,
    ) -> async_graphql::Result<Playlist> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        if title.trim().is_empty() {
            return Err(async_graphql::Error::new("title is required"));
        }
        let now = OffsetDateTime::now_utc();
        let playlist = Playlist {
            id: PlaylistId::new(),
            owner_id: principal.user_id,
            title,
            overview,
            created_at: now,
            updated_at: now,
        };
        storage.create_playlist(&playlist).await.map_err(gql_err)?;
        Ok(playlist)
    }

    /// Rename / re-annotate a playlist. Owner or admin only.
    async fn update_playlist(
        &self,
        ctx: &Context<'_>,
        id: PlaylistId,
        title: Option<String>,
        overview: Option<String>,
    ) -> async_graphql::Result<Playlist> {
        let storage = storage(ctx)?;
        let mut playlist = authed_playlist(ctx, storage, id).await?;
        if let Some(title) = title {
            if title.trim().is_empty() {
                return Err(async_graphql::Error::new("title cannot be empty"));
            }
            playlist.title = title;
        }
        if let Some(overview) = overview {
            playlist.overview = Some(overview);
        }
        playlist.updated_at = OffsetDateTime::now_utc();
        storage.update_playlist(&playlist).await.map_err(gql_err)?;
        Ok(playlist)
    }

    /// Delete a playlist and its entries. Returns it, or null when the id
    /// didn't exist. Owner or admin only.
    async fn delete_playlist(
        &self,
        ctx: &Context<'_>,
        id: PlaylistId,
    ) -> async_graphql::Result<Option<Playlist>> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let playlist_id = id;
        let Some(playlist) = storage.get_playlist(playlist_id).await.map_err(gql_err)? else {
            return Ok(None);
        };
        if !principal.is_admin && playlist.owner_id != principal.user_id {
            return Err(async_graphql::Error::new("playlist not found"));
        }
        storage
            .delete_playlist(playlist_id)
            .await
            .map_err(gql_err)?;
        Ok(Some(playlist))
    }

    /// Replace a playlist's entries wholesale. Every target must
    /// resolve; the error names the first id that doesn't. Owner or
    /// admin only.
    async fn set_playlist_items(
        &self,
        ctx: &Context<'_>,
        id: PlaylistId,
        items: Vec<PlayTargetInput>,
    ) -> async_graphql::Result<Playlist> {
        let storage = storage(ctx)?;
        let playlist = authed_playlist(ctx, storage, id).await?;
        let targets = resolve_targets(ctx, storage, &items).await?;
        storage
            .set_playlist_items(playlist.id, &targets)
            .await
            .map_err(gql_err)?;
        Ok(playlist)
    }

    /// Append entries to the end of a playlist. Same validation as
    /// `setPlaylistItems`. Owner or admin only.
    async fn add_playlist_items(
        &self,
        ctx: &Context<'_>,
        id: PlaylistId,
        items: Vec<PlayTargetInput>,
    ) -> async_graphql::Result<Playlist> {
        let storage = storage(ctx)?;
        let playlist = authed_playlist(ctx, storage, id).await?;
        let targets = resolve_targets(ctx, storage, &items).await?;
        storage
            .append_playlist_items(playlist.id, &targets)
            .await
            .map_err(gql_err)?;
        Ok(playlist)
    }
}
