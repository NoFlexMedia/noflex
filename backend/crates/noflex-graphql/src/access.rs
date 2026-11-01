// SPDX-License-Identifier: AGPL-3.0-only

use std::{collections::HashSet, sync::Arc};

use async_graphql::Context;
use noflex_core::{EditionId, MediaFileId, UserId, WorkId};
use noflex_storage::{RequestUser, Storage, domain::Work};

/// The authenticated caller, or "authentication required". Only the
/// request's [`RequestUser`] counts: the transport publishes it solely
/// for authenticated principals.
pub(crate) fn request_user(ctx: &Context<'_>) -> async_graphql::Result<RequestUser> {
    noflex_storage::access::request_user(ctx)
}

pub(crate) fn user(ctx: &Context<'_>) -> async_graphql::Result<UserId> {
    Ok(request_user(ctx)?.user_id)
}

/// The authenticated caller, refusing anyone who isn't an administrator.
/// The single admin gate for the GraphQL surface. `is_admin` is already
/// the stored role: the auth middleware resolves it from the user row
/// on every request rather than trusting the token claim.
pub(crate) fn require_admin(ctx: &Context<'_>) -> async_graphql::Result<RequestUser> {
    let user = request_user(ctx)?;
    if user.is_admin {
        Ok(user)
    } else {
        Err(async_graphql::Error::new("administrator required"))
    }
}

pub(crate) async fn can_work(
    storage: &Arc<dyn Storage>,
    user: RequestUser,
    work: WorkId,
) -> async_graphql::Result<bool> {
    noflex_storage::access::user_can_access_work(storage.as_ref(), user, work)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))
}

pub(crate) async fn visible_work(
    storage: &Arc<dyn Storage>,
    user: RequestUser,
    work: Work,
) -> async_graphql::Result<Option<Work>> {
    noflex_storage::access::visible_work(storage.as_ref(), user, work)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))
}

pub(crate) async fn can_edition(
    storage: &Arc<dyn Storage>,
    user: RequestUser,
    edition: EditionId,
) -> async_graphql::Result<bool> {
    noflex_storage::access::user_can_access_edition(storage.as_ref(), user, edition)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))
}

pub(crate) async fn can_media(
    storage: &Arc<dyn Storage>,
    user: RequestUser,
    media: MediaFileId,
) -> async_graphql::Result<bool> {
    noflex_storage::access::user_can_access_media_file(storage.as_ref(), user, media)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))
}

/// The subset of `works` and `editions` visible to `user`, resolved
/// with one batched edition read and one batched access check rather
/// than a query per id. Missing ids are simply absent from the result.
pub(crate) async fn visible_targets(
    storage: &Arc<dyn Storage>,
    user: RequestUser,
    works: &[WorkId],
    editions: &[EditionId],
) -> async_graphql::Result<(HashSet<WorkId>, HashSet<EditionId>)> {
    let gql = |e: noflex_storage::StorageError| async_graphql::Error::new(e.to_string());
    let edition_rows = if editions.is_empty() {
        Vec::new()
    } else {
        storage.get_editions(editions).await.map_err(gql)?
    };
    let mut candidates = works.to_vec();
    candidates.extend(edition_rows.iter().map(|edition| edition.work_id));
    candidates.sort_by_key(|id| id.into_uuid());
    candidates.dedup();
    let visible: HashSet<WorkId> = if candidates.is_empty() {
        HashSet::new()
    } else if user.is_admin {
        storage
            .get_works(&candidates)
            .await
            .map_err(gql)?
            .into_iter()
            .map(|work| work.id)
            .collect()
    } else {
        storage
            .accessible_work_ids(user.user_id, &candidates)
            .await
            .map_err(gql)?
            .into_iter()
            .collect()
    };
    let visible_editions = edition_rows
        .into_iter()
        .filter(|edition| visible.contains(&edition.work_id))
        .map(|edition| edition.id)
        .collect();
    let visible_works = works
        .iter()
        .copied()
        .filter(|id| visible.contains(id))
        .collect();
    Ok((visible_works, visible_editions))
}
