// SPDX-License-Identifier: AGPL-3.0-only

use async_graphql::Context;
use noflex_core::{EditionId, MediaFileId, SessionId, UserId, WorkId};

use crate::{Storage, domain::Work, error::StorageResult};

/// Authenticated caller attached to a GraphQL request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestUser {
    pub user_id: UserId,
    pub is_admin: bool,
}

/// The login session behind a GraphQL request, attached next to
/// [`RequestUser`] when the credential has one (API keys don't). Media
/// tokens minted for the request are bound to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallerSession(pub SessionId);

/// The request's [`CallerSession`], if any.
pub fn caller_session(ctx: &Context<'_>) -> Option<SessionId> {
    ctx.data_opt::<CallerSession>().map(|s| s.0)
}

impl RequestUser {
    pub fn access_user_id(self) -> Option<UserId> {
        (!self.is_admin).then_some(self.user_id)
    }
}

pub fn request_user(ctx: &Context<'_>) -> async_graphql::Result<RequestUser> {
    ctx.data_opt::<RequestUser>()
        .copied()
        .ok_or_else(|| async_graphql::Error::new("authentication required"))
}

pub async fn user_can_access_work(
    storage: &dyn Storage,
    user: RequestUser,
    work: WorkId,
) -> StorageResult<bool> {
    if user.is_admin {
        return Ok(storage.get_work(work).await?.is_some());
    }
    storage.user_can_access_work(user.user_id, work).await
}

pub async fn visible_work(
    storage: &dyn Storage,
    user: RequestUser,
    work: Work,
) -> StorageResult<Option<Work>> {
    Ok(user_can_access_work(storage, user, work.id)
        .await?
        .then_some(work))
}

pub async fn user_can_access_edition(
    storage: &dyn Storage,
    user: RequestUser,
    edition: EditionId,
) -> StorageResult<bool> {
    let Some(work) = storage.work_for_edition(edition).await? else {
        return Ok(false);
    };
    user_can_access_work(storage, user, work.id).await
}

pub async fn user_can_access_media_file(
    storage: &dyn Storage,
    user: RequestUser,
    media: MediaFileId,
) -> StorageResult<bool> {
    let Some(work) = storage.work_for_media_file(media).await? else {
        return Ok(false);
    };
    user_can_access_work(storage, user, work.id).await
}
