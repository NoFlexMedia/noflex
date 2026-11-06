// SPDX-License-Identifier: AGPL-3.0-only
//
// User management. Admin CRUD (`users`, `createUser`,
// `updateUser`, `deleteUser`, `userLibraryAccess`) gates on the
// per-request `Principal`; `changePassword` is self-service — it
// rehashes and revokes every session except the caller's, signing other
// devices out on their next request. The rest of what a viewer changes about
// themselves is in `profile`.

use std::sync::Arc;

use async_graphql::{Context, Object, SimpleObject};
use noflex_auth::Principal;
use noflex_core::{LibraryId, UserId, UserRole};
use noflex_storage::{
    Storage, StorageError,
    domain::{Library, Session, User},
};
use time::OffsetDateTime;

/// What `deleteUser` removed: the account and the sessions that went with it.
#[derive(SimpleObject)]
pub struct DeletedUser {
    pub user: User,
    pub sessions: Vec<Session>,
}

/// The calling principal, or "authentication required" when the
/// request carries no authenticated user.
pub(crate) fn authed_principal(ctx: &Context<'_>) -> async_graphql::Result<Principal> {
    let principal = ctx
        .data::<Principal>()
        .map_err(|_| async_graphql::Error::new("authentication required"))?;
    if !principal.is_authenticated() {
        return Err(async_graphql::Error::new("authentication required"));
    }
    Ok(*principal)
}

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

/// Live admin count — the guard against demoting or deleting the last
/// administrator. Reads fresh on every call; two concurrent admins
/// racing to demote each other can both pass it, which is acceptable
/// until the last-admin invariant moves into storage.
async fn admin_count(storage: &Arc<dyn Storage>) -> async_graphql::Result<usize> {
    let users = storage.list_users().await.map_err(gql_err)?;
    Ok(users.iter().filter(|u| u.is_admin).count())
}

#[derive(Default)]
pub struct UsersQuery;

#[Object]
impl UsersQuery {
    /// Every user account. Admin only.
    async fn users(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<User>> {
        crate::access::require_admin(ctx)?;
        storage(ctx)?.list_users().await.map_err(gql_err)
    }

    /// The calling user's own row, freshly read from storage rather
    /// than echoed back from token claims. Pair with
    /// `viewer { libraryAccess }` for the self-serve grant list.
    async fn viewer(&self, ctx: &Context<'_>) -> async_graphql::Result<User> {
        let principal = authed_principal(ctx)?;
        storage(ctx)?
            .get_user(principal.user_id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("user not found"))
    }

    /// The libraries a user has been explicitly granted. Admin only.
    async fn user_library_access(
        &self,
        ctx: &Context<'_>,
        user_id: UserId,
    ) -> async_graphql::Result<Vec<Library>> {
        crate::access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let uid = user_id;
        let ids = storage.list_user_libraries(uid).await.map_err(gql_err)?;
        let mut libs = Vec::with_capacity(ids.len());
        for id in ids {
            // A grant outliving its library is skipped, not errored —
            // the admin UI wants the truthful remainder.
            if let Some(lib) = storage.get_library(id).await.map_err(gql_err)? {
                libs.push(lib);
            }
        }
        Ok(libs)
    }
}

#[derive(Default)]
pub struct UsersMutation;

#[Object]
impl UsersMutation {
    /// Create a user. `is_admin` is derived from `role` so the two
    /// never drift apart. Admin only.
    async fn create_user(
        &self,
        ctx: &Context<'_>,
        username: String,
        password: String,
        role: UserRole,
    ) -> async_graphql::Result<User> {
        crate::access::require_admin(ctx)?;
        let storage = storage(ctx)?;
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
        let user = User {
            id: UserId::new(),
            username,
            display_name: None,
            password_hash: hash,
            is_admin: role == UserRole::Admin,
            role,
            created_at: OffsetDateTime::now_utc(),
            avatar_updated_at: None,
        };
        storage.upsert_user(&user).await.map_err(|e| match e {
            StorageError::Constraint(_) => async_graphql::Error::new("username already taken"),
            other => gql_err(other),
        })?;
        Ok(user)
    }

    /// Update a user's role and/or library grants. Both optional;
    /// `libraryIds` is the full desired set, diffed against the
    /// stored grants. Admin only.
    async fn update_user(
        &self,
        ctx: &Context<'_>,
        id: UserId,
        role: Option<UserRole>,
        library_ids: Option<Vec<LibraryId>>,
    ) -> async_graphql::Result<User> {
        crate::access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let uid = id;
        let mut user = storage
            .get_user(uid)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("user not found"))?;

        if let Some(role) = role {
            if user.is_admin && role != UserRole::Admin && admin_count(storage).await? <= 1 {
                return Err(async_graphql::Error::new(
                    "cannot demote the last administrator",
                ));
            }
            user.role = role;
            user.is_admin = role == UserRole::Admin;
            storage.upsert_user(&user).await.map_err(gql_err)?;
        }

        if let Some(library_ids) = library_ids {
            let mut wanted = Vec::with_capacity(library_ids.len());
            for id in &library_ids {
                let lib_id = *id;
                storage
                    .get_library(lib_id)
                    .await
                    .map_err(gql_err)?
                    .ok_or_else(|| {
                        async_graphql::Error::new(format!("library not found: {}", id.0))
                    })?;
                wanted.push(lib_id);
            }
            let current = storage.list_user_libraries(uid).await.map_err(gql_err)?;
            for lib in &wanted {
                if !current.contains(lib) {
                    storage
                        .grant_library_access(uid, *lib)
                        .await
                        .map_err(gql_err)?;
                }
            }
            for lib in &current {
                if !wanted.contains(lib) {
                    storage
                        .revoke_library_access(uid, *lib)
                        .await
                        .map_err(gql_err)?;
                }
            }
        }
        Ok(user)
    }

    /// Delete a user. Refuses the caller's own account and the last
    /// administrator; sessions cascade away with the row. Admin only.
    async fn delete_user(
        &self,
        ctx: &Context<'_>,
        id: UserId,
    ) -> async_graphql::Result<DeletedUser> {
        let principal = crate::access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let uid = id;
        if uid == principal.user_id {
            return Err(async_graphql::Error::new("cannot delete your own account"));
        }
        let user = storage
            .get_user(uid)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("user not found"))?;
        if user.is_admin && admin_count(storage).await? <= 1 {
            return Err(async_graphql::Error::new(
                "cannot delete the last administrator",
            ));
        }
        let sessions = storage.list_sessions_for_user(uid).await.map_err(gql_err)?;
        storage.delete_user(uid).await.map_err(gql_err)?;
        Ok(DeletedUser { user, sessions })
    }

    /// Change the caller's own password. Revokes every session except
    /// the caller's current one, signing other devices out on their next
    /// request. API keys are not revoked.
    // Credential checks cost most of the per-operation complexity budget,
    // so aliasing can't batch hundreds of guesses into one request.
    #[graphql(complexity = "crate::CREDENTIAL_COMPLEXITY")]
    async fn change_password(
        &self,
        ctx: &Context<'_>,
        current_password: String,
        new_password: String,
    ) -> async_graphql::Result<bool> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        if new_password.is_empty() {
            return Err(async_graphql::Error::new("new password is required"));
        }
        if current_password.len() > noflex_auth::MAX_PASSWORD_BYTES
            || new_password.len() > noflex_auth::MAX_PASSWORD_BYTES
        {
            return Err(async_graphql::Error::new("password is too long"));
        }
        let mut user = storage
            .get_user(principal.user_id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("user not found"))?;
        let throttle = crate::auth::login_throttle(ctx)?;
        let client_addr = ctx.data_opt::<noflex_auth::ClientAddr>().copied();
        throttle
            .check(&user.username, client_addr)
            .map_err(crate::auth::too_many_attempts)?;
        if noflex_auth::verify_password(&current_password, &user.password_hash)
            .await
            .is_err()
        {
            throttle.record_failure(&user.username, client_addr);
            return Err(async_graphql::Error::new("incorrect current password"));
        }
        throttle.record_success(&user.username);
        user.password_hash = noflex_auth::hash_password(&new_password)
            .await
            .map_err(|_| async_graphql::Error::new("password processing failed"))?;
        storage.upsert_user(&user).await.map_err(gql_err)?;
        let sessions = storage
            .list_sessions_for_user(user.id)
            .await
            .map_err(gql_err)?;
        for session in sessions {
            if Some(session.id) == principal.session_id {
                continue;
            }
            storage.revoke_session(session.id).await.map_err(gql_err)?;
        }
        Ok(true)
    }
}
