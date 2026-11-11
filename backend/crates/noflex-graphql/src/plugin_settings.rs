// SPDX-License-Identifier: AGPL-3.0-only
//
// Administrators editing the settings a plugin's manifest declares. The
// values go to storage, over config.toml, and the plugin reloads with them.

use std::sync::Arc;

use async_graphql::{Context, ErrorExtensions as _, InputObject, Object};
use noflex_plugin_host::{Plugin, PluginId, PluginRegistry};

use crate::access;

/// A plugin setting a mutation can't take: an unknown key, a value that
/// doesn't fit its kind, or clearing a required one nothing else sets.
pub const INVALID_PLUGIN_SETTING: &str = "INVALID_PLUGIN_SETTING";

/// One value for `setPluginSettings`.
#[derive(InputObject, Debug)]
pub struct PluginSettingInput {
    pub key: String,
    /// In the form `PluginSetting.value` takes (`true`, `42`). Null or empty
    /// clears the key, so config.toml or the default applies again.
    pub value: Option<String>,
}

#[derive(Default)]
pub struct PluginSettingsMutation;

#[Object]
impl PluginSettingsMutation {
    /// Set or clear several of a loaded plugin's settings at once and reload
    /// it with them. Every value is checked against the plugin's manifest
    /// first, and any that fails (`INVALID_PLUGIN_SETTING`) leaves all of
    /// them unchanged. Secret writes go in the audit log by key. Admin only.
    async fn set_plugin_settings(
        &self,
        ctx: &Context<'_>,
        plugin_id: String,
        values: Vec<PluginSettingInput>,
    ) -> async_graphql::Result<Plugin> {
        let admin = access::require_admin(ctx)?;
        let registry = ctx
            .data::<Arc<PluginRegistry>>()
            .map_err(|_| async_graphql::Error::new("plugin registry not configured"))?;
        let changes = values.into_iter().map(|v| (v.key, v.value)).collect();
        registry
            .update_settings(
                &PluginId(plugin_id.clone()),
                changes,
                &admin.user_id.to_string(),
            )
            .await
            .map_err(|e| {
                if e.is_bad_input() {
                    async_graphql::Error::new(e.to_string())
                        .extend_with(|_, ext| ext.set("code", INVALID_PLUGIN_SETTING))
                } else {
                    tracing::warn!(plugin = %plugin_id, error = %e, "saving plugin settings failed");
                    async_graphql::Error::new("saving plugin settings failed")
                }
            })
    }
}
