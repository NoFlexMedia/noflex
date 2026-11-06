// SPDX-License-Identifier: AGPL-3.0-only
//
// Session/device management. `mySessions` is the caller's own
// active-devices view; `sessions` is the admin cross-user view.
// `revokeSession` is owner-or-admin and answers "session not found"
// for missing rows and other users' rows alike, so non-admins can't
// probe session existence.

use std::sync::Arc;

use async_graphql::{Context, Object};
use noflex_core::{SessionId, UserId};
use noflex_storage::{Storage, domain::Session};

use crate::users::authed_principal;

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

#[derive(Default)]
pub struct SessionsQuery;

#[Object]
impl SessionsQuery {
    /// The caller's own active sessions/devices.
    async fn my_sessions(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<Session>> {
        let principal = authed_principal(ctx)?;
        storage(ctx)?
            .list_sessions_for_user(principal.user_id)
            .await
            .map_err(gql_err)
    }

    /// Active sessions across users — the admin active-device view.
    /// `userId` narrows to one user; omitted lists everyone.
    async fn sessions(
        &self,
        ctx: &Context<'_>,
        user_id: Option<UserId>,
    ) -> async_graphql::Result<Vec<Session>> {
        crate::access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        match user_id {
            Some(id) => storage.list_sessions_for_user(id).await.map_err(gql_err),
            None => storage.list_all_sessions().await.map_err(gql_err),
        }
    }
}

#[derive(Default)]
pub struct SessionsMutation;

#[Object]
impl SessionsMutation {
    /// Revoke one session: the caller's own, or anyone's as admin. Its
    /// refresh token and every access token minted from it stop working
    /// on their next request, since each request re-checks the session.
    /// Returns the revoked session.
    async fn revoke_session(
        &self,
        ctx: &Context<'_>,
        id: SessionId,
    ) -> async_graphql::Result<Session> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let session_id = id;
        let session = storage
            .get_session(session_id)
            .await
            .map_err(gql_err)?
            .filter(|s| principal.is_admin || s.user_id == principal.user_id)
            .ok_or_else(|| async_graphql::Error::new("session not found"))?;
        storage.revoke_session(session.id).await.map_err(gql_err)?;
        Ok(session)
    }

    /// Revoke every session a user holds; returns the revoked sessions.
    /// Admin only.
    async fn revoke_user_sessions(
        &self,
        ctx: &Context<'_>,
        user_id: UserId,
    ) -> async_graphql::Result<Vec<Session>> {
        crate::access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let sessions = storage
            .list_sessions_for_user(user_id)
            .await
            .map_err(gql_err)?;
        storage
            .revoke_user_sessions(user_id)
            .await
            .map_err(gql_err)?;
        Ok(sessions)
    }
}
