// SPDX-License-Identifier: AGPL-3.0-only
//
// Provider priority registry. Multiple plugins may export the same hook
// (e.g. TMDb + TVDB both implement `metadata` for movies). The host needs
// a deterministic order to fan out `search()` and to pick the primary
// `fetch()` target. This module owns that policy.
//
// Dispatch rules:
//   - Manifest declares per-hook defaults (`[priority] metadata = 100`).
//   - Server config overrides per (provider, work_kind) — see [`Overrides`].
//   - `priority = 0` means "disabled for this work kind".
//   - Sort key for candidate ranking is `(priority DESC, confidence DESC)`.
//   - Fetch is provider-exclusive — fallbacks only on failure, never field
//     merging across providers.

use std::collections::{HashMap, HashSet};

use async_graphql::SimpleObject;
use serde::{Deserialize, Serialize};

use crate::{HookKind, PluginId, PluginManifest, WorkKindFilter};

/// Per-(plugin, work-kind) priority. Higher = preferred. Zero = excluded.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Overrides {
    /// `overrides[plugin_id][hook][work_kind] = priority`. Missing entries
    /// fall back to the plugin's manifest defaults.
    #[serde(default)]
    pub by_plugin: HashMap<String, PluginOverride>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PluginOverride {
    /// `hook_name -> work_kind_name -> priority`. Work-kind names match
    /// the kebab-case enum spellings.
    #[serde(default, flatten)]
    pub priorities: HashMap<String, HashMap<String, i32>>,
}

/// Resolved priority table — what the dispatcher actually consults at
/// runtime. Built by composing the registry's current plugins with
/// [`Overrides`].
#[derive(Debug, Default, Clone)]
pub struct PriorityTable {
    inner: HashMap<(PluginId, HookKind, WorkKindFilter), i32>,
    /// Fallback per-hook priority when no work-kind override applies.
    hook_default: HashMap<(PluginId, HookKind), i32>,
    /// `inner` as built from manifests + config, so clearing an admin
    /// override restores the configured value instead of dropping it.
    configured: HashMap<(PluginId, HookKind, WorkKindFilter), i32>,
    /// Admin overrides from [`PriorityTable::set`], including ones for
    /// plugins not currently loaded (they apply when the plugin loads).
    admin: HashMap<(PluginId, HookKind, WorkKindFilter), i32>,
    /// Plugins the table was built over.
    loaded: HashSet<PluginId>,
}

/// One row of the active priority table as exposed to GraphQL
/// (`pluginOverrides` query, `setPluginOverride` mutation result).
#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "PluginOverrideEntry")]
pub struct PluginOverrideEntry {
    pub plugin_id: String,
    pub hook: HookKind,
    pub work_kind: WorkKindFilter,
    pub priority: i32,
    /// Set by an admin (`setPluginOverride`) rather than by a manifest or
    /// the server config; `clearPluginOverride` reverts it.
    pub admin_set: bool,
}

impl PriorityTable {
    /// Build a priority table from a set of plugin manifests + server overrides.
    pub fn build<'a, I>(plugins: I, overrides: &Overrides) -> Self
    where
        I: IntoIterator<Item = (&'a PluginId, &'a PluginManifest)>,
    {
        let mut inner: HashMap<(PluginId, HookKind, WorkKindFilter), i32> = HashMap::new();
        let mut hook_default: HashMap<(PluginId, HookKind), i32> = HashMap::new();
        let mut loaded = HashSet::new();

        for (id, manifest) in plugins {
            loaded.insert(id.clone());
            // Manifest's per-hook default.
            for (hook_name, pri) in &manifest.priority {
                if let Some(hook) = HookKind::parse(hook_name) {
                    hook_default.insert((id.clone(), hook), *pri);
                }
            }
            // Server overrides — per (plugin, hook, work_kind).
            if let Some(override_) = overrides.by_plugin.get(&id.0) {
                for (hook_name, per_kind) in &override_.priorities {
                    if let Some(hook) = HookKind::parse(hook_name) {
                        for (kind_name, pri) in per_kind {
                            if let Some(work_kind) = parse_work_kind(kind_name) {
                                inner.insert((id.clone(), hook, work_kind), *pri);
                            }
                        }
                    }
                }
            }
        }

        PriorityTable {
            configured: inner.clone(),
            inner,
            hook_default,
            admin: HashMap::new(),
            loaded,
        }
    }

    /// Resolve the effective priority for a (plugin, hook, work_kind) tuple.
    ///
    /// Lookup order:
    ///   1. Specific override for the work_kind.
    ///   2. Plugin's manifest default for the hook.
    ///   3. Zero (i.e. excluded — plugin doesn't claim this hook for this work).
    pub fn priority(&self, id: &PluginId, hook: HookKind, work_kind: WorkKindFilter) -> i32 {
        if let Some(&p) = self.inner.get(&(id.clone(), hook, work_kind)) {
            return p;
        }
        if let Some(&p) = self.hook_default.get(&(id.clone(), hook)) {
            return p;
        }
        0
    }

    /// Ranked list of plugins for a (hook, work_kind) — descending priority,
    /// ties broken by plugin ID (deterministic). Excludes priority == 0.
    pub fn ranked(&self, hook: HookKind, work_kind: WorkKindFilter) -> Vec<(PluginId, i32)> {
        let mut all: HashMap<PluginId, i32> = HashMap::new();
        for ((id, h, wk), pri) in &self.inner {
            if *h == hook && *wk == work_kind && *pri > 0 {
                all.insert(id.clone(), *pri);
            }
        }
        for ((id, h), pri) in &self.hook_default {
            if *h == hook && *pri > 0 && !all.contains_key(id) {
                all.insert(id.clone(), *pri);
            }
        }
        let mut v: Vec<_> = all.into_iter().collect();
        v.sort_by(|(a_id, a_pri), (b_id, b_pri)| b_pri.cmp(a_pri).then_with(|| a_id.cmp(b_id)));
        v
    }

    /// Ranked list of plugins for a hook with no work-kind dimension
    /// (e.g. `embedder`): hook-default priorities only, descending,
    /// ties broken by plugin ID. Excludes priority == 0.
    pub fn ranked_hook(&self, hook: HookKind) -> Vec<(PluginId, i32)> {
        let mut v: Vec<(PluginId, i32)> = self
            .hook_default
            .iter()
            .filter(|((_, h), pri)| *h == hook && **pri > 0)
            .map(|((id, _), pri)| (id.clone(), *pri))
            .collect();
        v.sort_by(|(a_id, a_pri), (b_id, b_pri)| b_pri.cmp(a_pri).then_with(|| a_id.cmp(b_id)));
        v
    }

    /// All (plugin, hook, work_kind) priority entries, in arbitrary
    /// order. The GraphQL `pluginOverrides` query uses this to render the
    /// active table for the admin UI.
    pub fn snapshot(&self) -> Vec<(PluginId, HookKind, WorkKindFilter, i32)> {
        self.inner
            .iter()
            .map(|((id, h, wk), p)| (id.clone(), *h, *wk, *p))
            .collect()
    }

    /// Snapshot of the active table as owned [`PluginOverrideEntry`]
    /// rows ready for GraphQL serialization.
    pub fn snapshot_gql(&self) -> Vec<PluginOverrideEntry> {
        self.inner
            .iter()
            .map(|((id, h, wk), p)| PluginOverrideEntry {
                plugin_id: id.0.clone(),
                hook: *h,
                work_kind: *wk,
                priority: *p,
                admin_set: self.admin.contains_key(&(id.clone(), *h, *wk)),
            })
            .collect()
    }

    /// Mutate the in-memory table in response to a
    /// `setPluginOverride` mutation. Hook + work-kind names are the
    /// kebab-case spellings the manifest already uses.
    pub fn set(
        &mut self,
        plugin_id: &str,
        hook: &str,
        work_kind: &str,
        priority: i32,
    ) -> Result<(), String> {
        let hook_kind = HookKind::parse(hook).ok_or_else(|| format!("unknown hook: {hook}"))?;
        let work_kind_filter =
            parse_work_kind(work_kind).ok_or_else(|| format!("unknown work kind: {work_kind}"))?;
        let key = (PluginId(plugin_id.to_string()), hook_kind, work_kind_filter);
        if self.loaded.contains(&key.0) {
            self.inner.insert(key.clone(), priority);
        }
        self.admin.insert(key, priority);
        Ok(())
    }

    /// Rebuild from the current plugin set and config — after a plugin
    /// was hot-loaded or removed — keeping admin overrides. Overrides for
    /// plugins that are no longer loaded are kept too, so they apply again
    /// if the plugin comes back.
    pub fn rebuild<'a, I>(&mut self, plugins: I, overrides: &Overrides)
    where
        I: IntoIterator<Item = (&'a PluginId, &'a PluginManifest)>,
    {
        let mut next = Self::build(plugins, overrides);
        for (key, &priority) in &self.admin {
            if next.loaded.contains(&key.0) {
                next.inner.insert(key.clone(), priority);
            }
        }
        next.admin = std::mem::take(&mut self.admin);
        *self = next;
    }

    /// Drop an admin override, falling back to the manifest/config
    /// priority (or the hook default when neither set one). Returns
    /// whether an admin override existed.
    pub fn clear(&mut self, plugin_id: &str, hook: &str, work_kind: &str) -> Result<bool, String> {
        let hook_kind = HookKind::parse(hook).ok_or_else(|| format!("unknown hook: {hook}"))?;
        let work_kind_filter =
            parse_work_kind(work_kind).ok_or_else(|| format!("unknown work kind: {work_kind}"))?;
        let key = (PluginId(plugin_id.to_string()), hook_kind, work_kind_filter);
        if self.admin.remove(&key).is_none() {
            return Ok(false);
        }
        match self.configured.get(&key) {
            Some(&priority) => self.inner.insert(key, priority),
            None => self.inner.remove(&key),
        };
        Ok(true)
    }
}

fn parse_work_kind(s: &str) -> Option<WorkKindFilter> {
    match s {
        "movie" => Some(WorkKindFilter::Movie),
        "series" => Some(WorkKindFilter::Series),
        "season" => Some(WorkKindFilter::Season),
        "episode" => Some(WorkKindFilter::Episode),
        "album" => Some(WorkKindFilter::Album),
        "track" => Some(WorkKindFilter::Track),
        "artist" => Some(WorkKindFilter::Artist),
        "audio-book" | "audio_book" => Some(WorkKindFilter::AudioBook),
        "photo" => Some(WorkKindFilter::Photo),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with_priority(id: &str, hook: &str, priority: i32) -> PluginManifest {
        let mut p = std::collections::BTreeMap::new();
        p.insert(hook.to_string(), priority);
        PluginManifest {
            manifest_version: 1,
            plugin_id: id.to_string(),
            display_name: id.to_string(),
            version: "0.1.0".into(),
            noflex_api_version: "0.2".into(),
            hooks: vec![hook.to_string()],
            capabilities: vec![],
            events: vec![],
            http_egress_hosts: vec![],
            secrets_read: vec![],
            user_secrets_read: vec![],
            user_secrets_write: vec![],
            author: None,
            license: None,
            homepage: None,
            priority: p,
            deadlines_secs: Default::default(),
            memory_limit_mb: None,
            tasks: vec![],
            embedding: None,
        }
    }

    #[test]
    fn manifest_defaults_apply_when_no_override() {
        let tmdb = (
            PluginId("tmdb".into()),
            manifest_with_priority("tmdb", "metadata", 100),
        );
        let tvdb = (
            PluginId("tvdb".into()),
            manifest_with_priority("tvdb", "metadata", 80),
        );
        let table = PriorityTable::build(
            [(&tmdb.0, &tmdb.1), (&tvdb.0, &tvdb.1)],
            &Overrides::default(),
        );
        assert_eq!(
            table.priority(&tmdb.0, HookKind::Metadata, WorkKindFilter::Movie),
            100
        );
        assert_eq!(
            table.priority(&tvdb.0, HookKind::Metadata, WorkKindFilter::Movie),
            80
        );

        let ranked = table.ranked(HookKind::Metadata, WorkKindFilter::Movie);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].0.0, "tmdb");
        assert_eq!(ranked[1].0.0, "tvdb");
    }

    #[test]
    fn server_override_per_work_kind_beats_manifest_default() {
        let tmdb = (
            PluginId("tmdb".into()),
            manifest_with_priority("tmdb", "metadata", 100),
        );
        let anidb = (
            PluginId("anidb".into()),
            manifest_with_priority("anidb", "metadata", 50),
        );

        // AniDB beats TMDb on series-classified-as-anime, but only for series.
        let mut overrides = Overrides::default();
        let mut anidb_pri = HashMap::new();
        anidb_pri.insert(
            "metadata".to_string(),
            [("series".to_string(), 250)].into_iter().collect(),
        );
        overrides.by_plugin.insert(
            "anidb".to_string(),
            PluginOverride {
                priorities: anidb_pri,
            },
        );

        let table = PriorityTable::build([(&tmdb.0, &tmdb.1), (&anidb.0, &anidb.1)], &overrides);
        let ranked_series = table.ranked(HookKind::Metadata, WorkKindFilter::Series);
        assert_eq!(ranked_series[0].0.0, "anidb");
        assert_eq!(ranked_series[0].1, 250);
        assert_eq!(ranked_series[1].0.0, "tmdb");
        assert_eq!(ranked_series[1].1, 100);

        // For movies, manifest default still applies — AniDB has no movie override.
        let ranked_movies = table.ranked(HookKind::Metadata, WorkKindFilter::Movie);
        assert_eq!(ranked_movies[0].0.0, "tmdb");
    }

    #[test]
    fn zero_priority_excludes_from_ranking() {
        let mut manifest = manifest_with_priority("tmdb", "metadata", 100);
        manifest.priority.insert("metadata".into(), 0);
        let id = PluginId("tmdb".into());
        let table = PriorityTable::build([(&id, &manifest)], &Overrides::default());
        assert!(
            table
                .ranked(HookKind::Metadata, WorkKindFilter::Movie)
                .is_empty()
        );
    }

    #[test]
    fn ranked_hook_ignores_work_kind_overrides() {
        let tmdb = (
            PluginId("tmdb".into()),
            manifest_with_priority("tmdb", "embedder", 100),
        );
        let local = (
            PluginId("local".into()),
            manifest_with_priority("local", "embedder", 200),
        );
        let table = PriorityTable::build(
            [(&tmdb.0, &tmdb.1), (&local.0, &local.1)],
            &Overrides::default(),
        );
        let ranked = table.ranked_hook(HookKind::Embedder);
        assert_eq!(ranked, vec![(local.0.clone(), 200), (tmdb.0.clone(), 100)]);
        assert!(table.ranked_hook(HookKind::Metadata).is_empty());
    }

    #[test]
    fn set_replaces_existing_entry() {
        let id = PluginId("tmdb".to_string());
        let manifest = manifest_with_priority("tmdb", "metadata", 100);
        let mut table = PriorityTable::build([(&id, &manifest)], &Overrides::default());
        assert!(table.set("tmdb", "metadata", "movie", 500).is_ok());
        let snap = table.snapshot();
        assert!(snap.iter().any(|(_, h, k, p)| {
            h == &HookKind::Metadata && k == &WorkKindFilter::Movie && *p == 500
        }));
    }

    #[test]
    fn clear_restores_the_configured_priority() {
        let id = PluginId("tmdb".to_string());
        let manifest = manifest_with_priority("tmdb", "metadata", 100);
        let mut overrides = Overrides::default();
        overrides.by_plugin.insert(
            "tmdb".into(),
            PluginOverride {
                priorities: HashMap::from([(
                    "metadata".to_string(),
                    HashMap::from([("movie".to_string(), 300)]),
                )]),
            },
        );
        let mut table = PriorityTable::build([(&id, &manifest)], &overrides);
        table.set("tmdb", "metadata", "movie", 500).unwrap();
        table.set("tmdb", "metadata", "series", 50).unwrap();
        assert!(table.snapshot_gql().iter().all(|e| e.admin_set));

        assert_eq!(table.clear("tmdb", "metadata", "movie"), Ok(true));
        assert_eq!(
            table.priority(&id, HookKind::Metadata, WorkKindFilter::Movie),
            300
        );
        assert_eq!(table.clear("tmdb", "metadata", "series"), Ok(true));
        assert_eq!(
            table.priority(&id, HookKind::Metadata, WorkKindFilter::Series),
            100
        );
        assert_eq!(table.clear("tmdb", "metadata", "series"), Ok(false));
        assert!(table.snapshot_gql().iter().all(|e| !e.admin_set));
    }

    #[test]
    fn rebuild_picks_up_new_plugins_and_keeps_admin_overrides() {
        let tmdb = (
            PluginId("tmdb".to_string()),
            manifest_with_priority("tmdb", "metadata", 100),
        );
        let tvdb = (
            PluginId("tvdb".to_string()),
            manifest_with_priority("tvdb", "metadata", 80),
        );
        let mut table = PriorityTable::build([(&tmdb.0, &tmdb.1)], &Overrides::default());
        table.set("tmdb", "metadata", "series", 10).unwrap();

        table.rebuild(
            [(&tmdb.0, &tmdb.1), (&tvdb.0, &tvdb.1)],
            &Overrides::default(),
        );
        let ranked: Vec<_> = table
            .ranked(HookKind::Metadata, WorkKindFilter::Series)
            .into_iter()
            .map(|(id, p)| (id.0, p))
            .collect();
        assert_eq!(ranked, [("tvdb".to_string(), 80), ("tmdb".to_string(), 10)]);

        // Unloaded plugins drop out of ranking; their override returns with them.
        table.rebuild([(&tvdb.0, &tvdb.1)], &Overrides::default());
        for kind in [WorkKindFilter::Movie, WorkKindFilter::Series] {
            assert!(
                table
                    .ranked(HookKind::Metadata, kind)
                    .iter()
                    .all(|(id, _)| id.0 == "tvdb")
            );
        }
        table.rebuild(
            [(&tmdb.0, &tmdb.1), (&tvdb.0, &tvdb.1)],
            &Overrides::default(),
        );
        assert_eq!(
            table.priority(&tmdb.0, HookKind::Metadata, WorkKindFilter::Series),
            10
        );
    }

    #[test]
    fn set_rejects_unknown_hook() {
        let id = PluginId("tmdb".to_string());
        let manifest = manifest_with_priority("tmdb", "metadata", 100);
        let mut table = PriorityTable::build([(&id, &manifest)], &Overrides::default());
        let err = table.set("tmdb", "bogus-hook", "movie", 100).unwrap_err();
        assert!(err.contains("unknown hook"), "msg: {err}");
    }
}
