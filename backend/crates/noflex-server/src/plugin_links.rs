// SPDX-License-Identifier: AGPL-3.0-only
//
// Server side of the per-user plugin link flow. `ServerPluginLinks`
// drives the registry's user-link hook calls and holds the pending
// states between `startPluginLink` and the provider's redirect to the
// (deliberately unauthenticated) callback route. The pending state is
// the credential: single-use, short-TTL, and bound to the linking
// user — the browser arrives via provider redirect without our session
// headers, so the callback route must not demand a Principal.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, anyhow};
use axum::response::{Html, IntoResponse, Response};
use noflex_core::UserId;
use noflex_graphql::{PluginLinkChallenge, PluginLinkService};
use noflex_plugin_host::{HookKind, LoadedPlugin, PluginId, PluginRegistry};
use noflex_storage::Storage;
use time::OffsetDateTime;

/// States handed out by `link_start` die after this; a user who sits on
/// the provider's consent page longer restarts the flow.
const PENDING_TTL: time::Duration = time::Duration::minutes(10);
const MAX_LINK_SECRETS: usize = 16;
const MAX_SECRET_KEY_BYTES: usize = 128;
const MAX_SECRET_VALUE_BYTES: usize = 64 * 1024;

/// The registry calls the link flow needs, behind a trait so tests can
/// drive [`ServerPluginLinks`] with a fake instead of a WASM fixture.
#[async_trait::async_trait]
pub trait PluginLinkHooks: Send + Sync {
    /// Begin the flow: the plugin must be loaded and export the
    /// user-link hook; returns its auth URL + state.
    async fn link_start(
        &self,
        plugin_id: &str,
        user_id: String,
    ) -> anyhow::Result<PluginLinkChallenge>;

    /// Finish the flow from the provider's raw callback query,
    /// returning the per-user secrets the host persists on the
    /// plugin's behalf.
    async fn link_complete(
        &self,
        plugin_id: &str,
        user_id: String,
        state: String,
        callback_query: String,
    ) -> anyhow::Result<Vec<(String, String)>>;

    /// Manifest keys the host may persist from link completion.
    fn writable_secret_keys(&self, plugin_id: &str) -> anyhow::Result<Vec<String>>;
}

fn link_plugin(registry: &PluginRegistry, plugin_id: &str) -> anyhow::Result<Arc<LoadedPlugin>> {
    let plugin = registry
        .get_plugin(&PluginId(plugin_id.to_string()))
        .ok_or_else(|| anyhow!("plugin not loaded: {plugin_id}"))?;
    if !plugin.exports(HookKind::UserLink) {
        return Err(anyhow!(
            "plugin {plugin_id} does not export the user-link hook"
        ));
    }
    Ok(plugin)
}

#[async_trait::async_trait]
impl PluginLinkHooks for PluginRegistry {
    async fn link_start(
        &self,
        plugin_id: &str,
        user_id: String,
    ) -> anyhow::Result<PluginLinkChallenge> {
        let plugin = link_plugin(self, plugin_id)?;
        match plugin.call_link_start(user_id).await {
            Ok(Ok(challenge)) => Ok(PluginLinkChallenge {
                auth_url: challenge.auth_url,
                state: challenge.state,
            }),
            Ok(Err(e)) => Err(anyhow!("plugin {plugin_id} rejected link start: {e:?}")),
            Err(e) => Err(e).context("link start dispatch failed"),
        }
    }

    async fn link_complete(
        &self,
        plugin_id: &str,
        user_id: String,
        state: String,
        callback_query: String,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let plugin = link_plugin(self, plugin_id)?;
        match plugin
            .call_link_complete(user_id, state, callback_query)
            .await
        {
            Ok(Ok(secrets)) => Ok(secrets.into_iter().map(|s| (s.key, s.value)).collect()),
            Ok(Err(e)) => Err(anyhow!("plugin {plugin_id} rejected link complete: {e:?}")),
            Err(e) => Err(e).context("link complete dispatch failed"),
        }
    }

    fn writable_secret_keys(&self, plugin_id: &str) -> anyhow::Result<Vec<String>> {
        let plugin = link_plugin(self, plugin_id)?;
        if !plugin
            .manifest
            .capabilities
            .iter()
            .any(|capability| capability == "user-secrets.write")
        {
            return Err(anyhow!(
                "plugin {plugin_id} does not declare `user-secrets.write`"
            ));
        }
        Ok(plugin.manifest.user_secrets_write.clone())
    }
}

struct PendingLink {
    user: UserId,
    plugin_id: String,
    expires_at: OffsetDateTime,
}

/// The link-flow service behind `startPluginLink` and the callback
/// route. Construct with the plugin registry (or a test fake) and
/// storage; hand to the schema as `Arc<dyn PluginLinkService>` and
/// keep the concrete `Arc` for the callback route.
pub struct ServerPluginLinks {
    hooks: Arc<dyn PluginLinkHooks>,
    storage: Arc<dyn Storage>,
    /// In-flight flows by state. Swept lazily on access — no background
    /// reaper: a forgotten entry costs a few bytes until the next touch.
    pending: Mutex<HashMap<String, PendingLink>>,
}

impl ServerPluginLinks {
    pub fn new(hooks: Arc<dyn PluginLinkHooks>, storage: Arc<dyn Storage>) -> Self {
        Self {
            hooks,
            storage,
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Consume the pending state and finish the flow: the plugin
    /// exchanges the raw callback query for secrets, which the host
    /// persists under the linking user and audit-logs. The state is
    /// removed *before* any validation, so a replay — or any failed
    /// attempt — always kills it. Missing, expired, and
    /// plugin-mismatched states all fail with one generic error.
    pub async fn complete_link(
        &self,
        plugin_id: &str,
        state: &str,
        callback_query: &str,
    ) -> anyhow::Result<()> {
        let pending = {
            let mut map = self.pending.lock().expect("pending links poisoned");
            sweep_expired(&mut map);
            map.remove(state)
        };
        let pending = pending
            .filter(|p| p.plugin_id == plugin_id)
            .ok_or_else(|| anyhow!("link failed or expired"))?;
        let writable_keys = self.hooks.writable_secret_keys(plugin_id)?;
        let secrets = self
            .hooks
            .link_complete(
                plugin_id,
                pending.user.to_string(),
                state.to_string(),
                callback_query.to_string(),
            )
            .await?;
        if secrets.len() > MAX_LINK_SECRETS {
            return Err(anyhow!("plugin returned too many link secrets"));
        }
        for (key, value) in &secrets {
            if !writable_keys.iter().any(|declared| declared == key) {
                return Err(anyhow!(
                    "plugin returned undeclared writable secret `{key}`"
                ));
            }
            if key.is_empty()
                || key.len() > MAX_SECRET_KEY_BYTES
                || value.len() > MAX_SECRET_VALUE_BYTES
            {
                return Err(anyhow!("plugin returned an invalid link secret"));
            }
        }
        for (key, value) in secrets {
            self.storage
                .set_user_plugin_secret(pending.user, plugin_id, &key, &value)
                .await?;
            noflex_plugin_host::audit::user_secret_write(
                plugin_id,
                &pending.user.to_string(),
                &key,
            );
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl PluginLinkService for ServerPluginLinks {
    async fn start_link(
        &self,
        user: UserId,
        plugin_id: &str,
    ) -> anyhow::Result<PluginLinkChallenge> {
        let challenge = self.hooks.link_start(plugin_id, user.to_string()).await?;
        let mut map = self.pending.lock().expect("pending links poisoned");
        sweep_expired(&mut map);
        map.insert(
            challenge.state.clone(),
            PendingLink {
                user,
                plugin_id: plugin_id.to_string(),
                expires_at: OffsetDateTime::now_utc() + PENDING_TTL,
            },
        );
        Ok(challenge)
    }
}

fn sweep_expired(map: &mut HashMap<String, PendingLink>) {
    let now = OffsetDateTime::now_utc();
    map.retain(|_, p| p.expires_at > now);
}

/// Shared body of the callback route, so `main.rs` and tests mount the
/// same logic. `state` is the parsed `state` query parameter;
/// `raw_query` is the undecoded query string the plugin needs for its
/// code exchange. Every failure collapses to one generic 400 — the
/// response must not reveal which stage failed.
pub async fn callback_response(
    links: &ServerPluginLinks,
    plugin_id: &str,
    raw_query: Option<&str>,
    state: Option<&str>,
) -> Response {
    let generic = || {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "link failed or expired",
        )
            .into_response()
    };
    let Some(state) = state else {
        return generic();
    };
    match links
        .complete_link(plugin_id, state, raw_query.unwrap_or_default())
        .await
    {
        Ok(()) => Html("Account linked — you can close this tab.").into_response(),
        Err(e) => {
            tracing::warn!(plugin = plugin_id, error = %e, "plugin link callback failed");
            generic()
        }
    }
}
