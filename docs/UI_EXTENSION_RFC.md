<!-- SPDX-License-Identifier: AGPL-3.0-only -->
# RFC: NoFlex UI Extensions

**Status: Draft proposal.** None of the APIs described here is implemented by this document.

This RFC sketches independently built, trusted UI bundles with explicit contribution points. All APIs and package names below are proposed.

## 1. Goals and boundaries

An extension can:

- Add pages, settings, contextual panels, and actions.
- Use supported NoFlex components and runtime services.
- Query GraphQL through the host's authenticated transport.
- Install independently without rebuilding the server or core web app.

An extension cannot rely on internal Svelte components or stores, core Houdini-generated artifacts, undocumented DOM structure or CSS classes, or patching existing routes or application functions.

Frontend bundles execute as native application JavaScript. Installing one grants full browser-side trust. API restrictions are compatibility contracts, not a security sandbox. Backend code remains subject to the existing WASM boundary.

## 2. Package and discovery

A package contains:

```text
trakt/
  manifest.toml
  backend.wasm           # optional
  ui/
    entry.js
    assets/
```

One authoring manifest supplies metadata; tooling generates any embedded WASM metadata and browser-facing descriptors. There are no hand-maintained duplicate manifests.

Example UI declaration:

```toml
[ui]
sdk_version = "0.1.0"
entry = "ui/entry.js"

[[ui.contributions]]
id = "activity"
kind = "panel"
target = "noflex/work/details/panels/v1"
title = "Trakt activity"
work_kinds = ["movie", "episode"]

[[ui.contributions]]
id = "sync"
kind = "action"
target = "noflex/work/actions/v1"
title = "Sync with Trakt"
```

Contribution IDs are local to the package; the host namespaces them as `trakt:activity`, for example.

The server exposes permission-filtered contribution descriptors through GraphQL. Discovering available contributions does not require executing JavaScript.

## 3. Named extension points

Each point specifies supported contribution kinds and typed context.

| Extension point | Contribution kind | Context |
| --- | --- | --- |
| `noflex/extensions/pages/v1` | Page | Route state |
| `noflex/settings/admin/v1` | Settings view | Administrator scope |
| `noflex/settings/account/v1` | Settings view | Current account |
| `noflex/work/details/panels/v1` | Panel | Work reference |
| `noflex/work/actions/v1` | Action | Work reference |
| `noflex/edition/actions/v1` | Action | Edition reference |

Contexts contain readonly identifiers and small presentation hints, not storage objects or live application stores.

```ts
interface WorkContext {
  readonly workId: WorkId;
  readonly libraryId: LibraryId;
  readonly kind: WorkKind;
}
```

Work, Edition, and MediaFile contexts remain distinct. The host owns placement, grouping, ordering, overflow menus, and surrounding chrome. Plugins do not request arbitrary screen coordinates or DOM selectors.

## 4. Registration API

The build tool generates a TypeScript contract from the manifest. The entry module supplies implementations for declared contributions:

```ts
import { defineUiPlugin } from "@noflex/plugin-api";
import type { UiContract } from "./generated/ui-contract";
import { SyncTrakt } from "./generated/operations";

export default defineUiPlugin<UiContract>({
  panels: {
    activity: () => import("./ActivityPanel.svelte"),
  },

  actions: {
    sync: async ({ context, api, signal }) => {
      await api.graphql.execute(
        SyncTrakt,
        { workId: context.workId },
        { signal },
      );
    },
  },
});
```

The generated contract checks that every declaration has an implementation, no undeclared implementation is registered, context types are correct, panel components are compatible, and action handlers have the expected asynchronous form.

`SyncTrakt` illustrates a hypothetical plugin operation, not an existing core mutation.

Actions are host-rendered controls, not plugin-rendered buttons embedded into menus. The host handles pending state, duplicate-click prevention, and error presentation. Visibility filters are declarative where possible; they improve presentation but never replace backend authorization.

## 5. Public SDK

Two public packages form the initial supported surface.

### @noflex/plugin-api

Provides:

- Contribution and context types.
- Authenticated GraphQL transport.
- Navigation requests.
- Notifications and confirmation dialogs.
- Locale and theme information.
- Lifecycle cleanup and cancellation.

It does not expose tokens, `Storage`, server secrets, internal stores, or raw cache objects.

### @noflex/ui

A deliberately limited supported component library provides:

- Buttons, fields, and form controls.
- Tabs, menus, dialogs, and tooltips.
- Loading, empty, and error states.
- Documented typography, spacing, and semantic theme tokens.

Plugin styles are scoped to their contribution. Global resets and styling unrelated host elements are unsupported. The goal is one supported toolkit, not publishing every internal component as permanent API.

## 6. GraphQL and Houdini

GraphQL remains the data contract. The SDK transport handles authentication and refresh; it is not a second plugin RPC system.

Each frontend package generates against the schema it uses:

```text
Core schema
    → core Houdini artifacts

Core schema + plugin schema
    → plugin client artifacts
```

An extension may use Houdini through an official transport adapter, but must not import the core application's `$houdini` output. Initially, caches are independent. Cross-view refresh uses a documented invalidation service, for example notifying the host that a particular Work should be refreshed, rather than directly manipulating caches.

Custom plugin GraphQL fields require a separate backend schema-contribution contract. This UI specification does not create that capability implicitly. The companion contract must establish namespace ownership, resolver dispatch, authorization, and schema compatibility. Until then, UI extensions consume existing core operations.

## 7. Loading, lifecycle, and compatibility

The host must:

- Validate package and SDK compatibility.
- Index contribution metadata.
- Load implementation bundles lazily.
- Mount components with typed context and an abort signal.
- Remove registrations, unmount views, unsubscribe listeners, and abort host-managed requests on unload.

Partial activation rolls back registrations. Cleanup is idempotent.

Ordinary contribution failures produce local error and retry states. Native code cannot offer hard failure isolation or undo completed mutations.

Bundles are served under content-addressed URLs. Backend, UI, and schema contributions activate as one validated package version, avoiding mixed releases.

The official build tool externalizes supported host packages and the Svelte runtime. It records a compatibility fingerprint covering the SDK, UI library, and compiler-runtime combination, not merely the Svelte major version.

Pre-1.0 compatibility versions move together exactly. Long-term compatibility is not promised before the contracts settle.

## 8. Developer workflow

One development command should:

- Link the package into a running development instance.
- Run the frontend watcher with hot reload.
- Build and reload WASM when present.
- Generate manifest and GraphQL types.
- Report compatibility and registration errors immediately.

Authors should not need to configure runtime import maps or write authentication plumbing themselves.

## 9. First release versus later

The first release includes:

- Core-defined pages, settings, panels, and actions.
- Shared components and authenticated transport.
- Independent installation.
- Typed registration and reliable cleanup.
- A first-party feature using exactly this API.

Later work includes:

- Plugin-defined extension points.
- Components exposed for other plugins.
- Typed plugin-to-plugin services.
- Dependency ordering and optional-provider behavior.

Plugin-owned namespaces are reserved now, but the entire ecosystem should not be built before the first vertical slice works.

The central contract is: declare a contribution, receive typed context, use supported services. Native Svelte rendering is the implementation mechanism underneath that contract, not permission to depend on the whole application.
