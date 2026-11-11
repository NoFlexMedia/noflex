// SPDX-License-Identifier: AGPL-3.0-only
//
// User management. Admin CRUD (`users`, `createUser`,
// `updateUser`, `deleteUser`, `userLibraryAccess`) gates on the
// per-request `Principal`; `changePassword` is self-service — it
// rehashes and revokes every session except the caller's, signing other
// devices out on their next request. `deleteAccount` is the caller leaving
// for good, and the last administrator's takes the server with it. The rest of
// what a viewer changes about themselves is in `profile`.

use std::{collections::BTreeSet, path::Path, sync::Arc};

use async_graphql::{Context, ErrorExtensions as _, Object, SimpleObject};
use noflex_auth::Principal;
use noflex_core::{LibraryId, UserId, UserRole};
use noflex_storage::{
    Storage, StorageError,
    domain::{Library, ServerReset, Session, User},
};
use time::OffsetDateTime;

use crate::erasure::Erasure;

/// `extensions.code` of a deletion that would erase the server, asked for
/// without `eraseServer`.
const LAST_ADMINISTRATOR: &str = "LAST_ADMINISTRATOR";

/// What `deleteUser` removed: the account and the sessions that went with it.
#[derive(SimpleObject)]
pub struct DeletedUser {
    pub user: User,
    pub sessions: Vec<Session>,
}

/// What `deleteAccount` removed.
#[derive(SimpleObject)]
pub struct DeletedAccount {
    /// The account, as it was.
    pub user: User,
    /// Whether it was the last administrator's, so the whole server is being
    /// erased: every account it holds is gone, and it comes back at first-run
    /// setup. Watch `serverInfo { resetting isConfigured }`.
    pub server_erased: bool,
}

/// What deleting their own account asks of the viewer, and what it takes.
#[derive(SimpleObject)]
pub struct AccountDeletion {
    /// Whether `deleteAccount` takes their password. An account that only signs
    /// in through another service has none, and confirms with its username.
    pub password_required: bool,
    /// Whether they're the last administrator, so deleting their account erases
    /// the whole server: every account, library, setting and cache, keeping
    /// only media files and the server's configuration.
    pub erases_server: bool,
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

/// Whether `user` is an administrator nobody else shares the role with.
async fn is_last_admin(storage: &Arc<dyn Storage>, user: &User) -> async_graphql::Result<bool> {
    Ok(user.is_admin && admin_count(storage).await? <= 1)
}

fn last_administrator() -> async_graphql::Error {
    async_graphql::Error::new(
        "you're the last administrator, so deleting your account erases the whole server",
    )
    .extend_with(|_, ext| ext.set("code", LAST_ADMINISTRATOR))
}

/// Tell the purge worker, and the smart transcoding planner, whose devices and
/// marks just went: it drops jobs planned for them alone at once, and finished
/// copies are swept up once nothing has wanted them for a day.
fn deleted(ctx: &Context<'_>) {
    if let Ok(erasure) = ctx.data::<Arc<Erasure>>() {
        erasure.request_purge();
    }
    if let Some(wake) = ctx.data_opt::<Arc<tokio::sync::Notify>>() {
        wake.notify_one();
    }
}

/// Folders outside the configuration that hold optimized versions: where an
/// administrator pointed smart transcoding, now and before.
async fn optimized_folders(storage: &Arc<dyn Storage>) -> async_graphql::Result<Vec<String>> {
    let mut folders = BTreeSet::new();
    if let Some(dir) = storage
        .smart_transcoding_settings()
        .await
        .map_err(gql_err)?
        .dir
    {
        folders.insert(dir);
    }
    for version in storage.list_optimized_versions().await.map_err(gql_err)? {
        if let Some(parent) = version
            .path
            .as_deref()
            .and_then(|path| Path::new(path).parent())
        {
            folders.insert(parent.to_string_lossy().into_owned());
        }
    }
    Ok(folders.into_iter().collect())
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

    /// What deleting their own account asks of the viewer, and whether it
    /// erases the server.
    async fn account_deletion(&self, ctx: &Context<'_>) -> async_graphql::Result<AccountDeletion> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let user = storage
            .get_user(principal.user_id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("user not found"))?;
        Ok(AccountDeletion {
            password_required: !user.password_hash.is_empty(),
            erases_server: is_last_admin(storage, &user).await?,
        })
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
            has_pin: false,
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

    /// Delete a user, as `deleteAccount` does: everything of theirs goes, and
    /// what can't go at once is purged in the background. Refuses the caller's
    /// own account, which is `deleteAccount`'s, and the last administrator.
    /// Admin only.
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
        match storage.delete_account(uid, OffsetDateTime::now_utc()).await {
            Ok(Some(_)) => {}
            Ok(None) => return Err(async_graphql::Error::new("user not found")),
            Err(StorageError::Conflict(_)) => {
                return Err(async_graphql::Error::new(
                    "cannot delete the last administrator",
                ));
            }
            Err(other) => return Err(gql_err(other)),
        }
        deleted(ctx);
        Ok(DeletedUser { user, sessions })
    }

    /// Delete the caller's own account, for good. Confirmed as signing in is:
    /// with `currentPassword`, throttled the same way, or for an account with
    /// none, by typing their `username`.
    ///
    /// Every session and API key of theirs goes before this answers, with
    /// their history, devices, playlists, invites, linked services and
    /// servers, picture and offline copies; what's left in the activity log
    /// and on disk is purged in the background.
    ///
    /// The last administrator's account only goes with the server. Without
    /// `eraseServer` that's refused with code `LAST_ADMINISTRATOR`; with it,
    /// everyone is signed out, the server restarts to erase every account,
    /// library, setting and cache it holds, keeping media files and its
    /// configuration, and comes back at first-run setup.
    // Credential checks cost most of the per-operation complexity budget,
    // so aliasing can't batch hundreds of guesses into one request.
    #[graphql(complexity = "crate::CREDENTIAL_COMPLEXITY")]
    async fn delete_account(
        &self,
        ctx: &Context<'_>,
        current_password: Option<String>,
        username: Option<String>,
        #[graphql(default)] erase_server: bool,
    ) -> async_graphql::Result<DeletedAccount> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let user = storage
            .get_user(principal.user_id)
            .await
            .map_err(gql_err)?
            .ok_or_else(|| async_graphql::Error::new("user not found"))?;
        if user.password_hash.is_empty() {
            if username.as_deref().map(str::trim) != Some(user.username.as_str()) {
                return Err(async_graphql::Error::new("type your username to confirm"));
            }
        } else {
            crate::profile::confirm_password(ctx, &user, current_password.as_deref().unwrap_or(""))
                .await?;
        }

        if is_last_admin(storage, &user).await? {
            if !erase_server {
                return Err(last_administrator());
            }
            let erasure = ctx
                .data::<Arc<Erasure>>()
                .map_err(|_| async_graphql::Error::new("server reset unavailable"))?;
            let reset = ServerReset {
                folders: optimized_folders(storage).await?,
                requested_at: OffsetDateTime::now_utc(),
            };
            // Up first, so nothing signs in between the sessions going and
            // the restart.
            erasure.set_resetting(true);
            match storage.request_server_reset(user.id, &reset).await {
                Ok(true) => {}
                Ok(false) => {
                    erasure.set_resetting(false);
                    return Err(async_graphql::Error::new(
                        "another administrator appeared; try again",
                    ));
                }
                Err(error) => {
                    erasure.set_resetting(false);
                    return Err(gql_err(error));
                }
            }
            tracing::warn!(user = %user.id, "the last administrator deleted their account; erasing the server");
            erasure.request_restart();
            return Ok(DeletedAccount {
                user,
                server_erased: true,
            });
        }

        match storage
            .delete_account(user.id, OffsetDateTime::now_utc())
            .await
        {
            Ok(Some(_)) => {}
            Ok(None) => return Err(async_graphql::Error::new("user not found")),
            // Made the last administrator since the check above.
            Err(StorageError::Conflict(_)) => return Err(last_administrator()),
            Err(other) => return Err(gql_err(other)),
        }
        deleted(ctx);
        Ok(DeletedAccount {
            user,
            server_erased: false,
        })
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
