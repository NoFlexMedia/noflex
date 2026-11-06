// SPDX-License-Identifier: AGPL-3.0-only
//
// Per-user plugin account linking (Trakt-style OAuth) and the admin
// "sync now" task enqueue. The link dance itself lives in the plugin:
// it hands the host an auth URL + state, the user authorizes
// out-of-band, and the provider redirects to the server's callback
// route, which completes the flow through the same service the
// `startPluginLink` mutation delegates to. The host persists the
// returned per-user secrets; their values never cross this surface —
// `myPluginLinks` exposes key names only.

use std::sync::Arc;

use async_graphql::{Context, Object, SimpleObject};
use noflex_core::UserId;
use noflex_plugin_host::PluginRegistry;
use noflex_storage::{Storage, domain::UserPluginSecret};
use time::OffsetDateTime;

use crate::users::authed_principal;

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn registry<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<PluginRegistry>> {
    ctx.data::<Arc<PluginRegistry>>()
        .map_err(|_| async_graphql::Error::new("plugin registry not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

/// The start of a plugin link flow: send the user to `auth_url`, then
/// wait for the provider to redirect back carrying `state`. The state
/// doubles as the single-use credential on the callback route.
#[derive(SimpleObject, Debug, Clone)]
pub struct PluginLinkChallenge {
    pub auth_url: String,
    pub state: String,
}

/// The calling user's link to one plugin: which credential keys are
/// stored and when the newest was written. Values are deliberately
/// absent — secrets must never cross the GraphQL surface, only the
/// fact that a key exists.
#[derive(SimpleObject, Debug, Clone)]
pub struct PluginLink {
    pub plugin_id: String,
    pub keys: Vec<String>,
    pub updated_at: OffsetDateTime,
}

impl PluginLink {
    /// The link these secrets make up; `None` for no secrets, as nothing is
    /// linked then.
    fn from_secrets(plugin_id: String, secrets: Vec<UserPluginSecret>) -> Option<Self> {
        let updated_at = secrets.iter().map(|secret| secret.updated_at).max()?;
        Some(Self {
            plugin_id,
            keys: secrets.into_iter().map(|secret| secret.key).collect(),
            updated_at,
        })
    }
}

/// Server-side link-flow driver, implemented by `noflex-server` and
/// wired into [`crate::SchemaContext`]. Behind a trait so the schema
/// crate never touches the plugin registry's dispatch internals — and
/// so tests can stub the whole flow.
#[async_trait::async_trait]
pub trait PluginLinkService: Send + Sync {
    /// Begin the link flow for `user` with `plugin_id`. Errors when the
    /// plugin isn't loaded or doesn't export the user-link hook.
    async fn start_link(
        &self,
        user: UserId,
        plugin_id: &str,
    ) -> anyhow::Result<PluginLinkChallenge>;
}

#[derive(Default)]
pub struct PluginLinksQuery;

#[Object]
impl PluginLinksQuery {
    /// Every loaded plugin the calling user has stored credentials
    /// for, with the credential key names. Plugins never linked — or
    /// since unlinked — are absent, as are stored secrets for plugins
    /// no longer loaded.
    async fn my_plugin_links(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<PluginLink>> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let registry = registry(ctx)?;
        let mut links = Vec::new();
        for plugin in registry.snapshot() {
            let secrets = storage
                .list_user_plugin_secrets(principal.user_id, &plugin.id)
                .await
                .map_err(gql_err)?;
            links.extend(PluginLink::from_secrets(plugin.id, secrets));
        }
        Ok(links)
    }
}

#[derive(Default)]
pub struct PluginLinksMutation;

#[Object]
impl PluginLinksMutation {
    /// Begin linking the calling user's account with a plugin
    /// (Trakt-style OAuth). Returns the provider auth URL to open; the
    /// flow completes when the provider redirects to the server's
    /// `/plugins/{pluginId}/link/callback` route.
    async fn start_plugin_link(
        &self,
        ctx: &Context<'_>,
        plugin_id: String,
    ) -> async_graphql::Result<PluginLinkChallenge> {
        let principal = authed_principal(ctx)?;
        let links = ctx
            .data::<Arc<dyn PluginLinkService>>()
            .map_err(|_| async_graphql::Error::new("plugin linking not configured"))?;
        links
            .start_link(principal.user_id, &plugin_id)
            .await
            .map_err(gql_err)
    }

    /// Unlink the calling user's account from a plugin by dropping
    /// every stored credential. Silent: the plugin is not called — it
    /// finds out on its next `read-for-user`, which starts missing.
    /// Returns the link, or null when nothing was linked.
    async fn unlink_plugin_account(
        &self,
        ctx: &Context<'_>,
        plugin_id: String,
    ) -> async_graphql::Result<Option<PluginLink>> {
        let principal = authed_principal(ctx)?;
        let storage = storage(ctx)?;
        let secrets = storage
            .list_user_plugin_secrets(principal.user_id, &plugin_id)
            .await
            .map_err(gql_err)?;
        storage
            .delete_user_plugin_secrets(principal.user_id, &plugin_id)
            .await
            .map_err(gql_err)?;
        Ok(PluginLink::from_secrets(plugin_id, secrets))
    }

    /// Enqueue a dynamic payload for one of a plugin's
    /// manifest-registered background tasks. Admin only. User-initiated
    /// "sync now" flows put the initiating user id in the payload by
    /// convention, so the task knows whose data to pull.
    async fn enqueue_plugin_task(
        &self,
        ctx: &Context<'_>,
        plugin_id: String,
        task: String,
        payload: Option<String>,
    ) -> async_graphql::Result<bool> {
        crate::access::require_admin(ctx)?;
        registry(ctx)?
            .enqueue_task(&plugin_id, &task, payload)
            .await
            .map_err(gql_err)?;
        Ok(true)
    }
}
