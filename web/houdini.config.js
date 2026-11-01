// SPDX-License-Identifier: AGPL-3.0-only

/** @type {import("houdini").ConfigFile} */
export default {
  // The server's committed SDL snapshot, so every document is validated
  // against the real API at build time.
  schemaPath: "../backend/crates/noflex-graphql/tests/snapshots/schema.graphql",
  url: "/graphql",
  watchSchema: false,
  runtimeDir: ".houdini",
  // Paint whatever the cache holds, then refresh it: instant back/forward
  // navigation without serving stale data for long.
  defaultCachePolicy: "CacheAndNetwork",
  // Fragments are shared selections; components read plain data.
  defaultFragmentMasking: "disable",
  scalars: {
    DateTime: { type: "string" },
    JSON: { type: "any" },
  },
  // Types without an `id`. Ones keyed here are normalised across queries;
  // the rest (UserData, TagSummary, PlaylistItem, SearchHit, …) are
  // embedded in their parent, which is the only place they're addressable.
  types: {
    HistoryRow: { keys: ["targetKind", "targetId"] },
    UserPreference: { keys: ["userId", "key"] },
    UserRestrictions: { keys: ["userId"] },
    ExternalIdentity: { keys: ["pluginId", "subject"] },
    PluginLink: { keys: ["pluginId"] },
    PluginOverrideEntry: { keys: ["pluginId", "hook", "workKind"] },
  },
  plugins: {
    "houdini-svelte": { static: true, forceRunesMode: true },
  },
};
