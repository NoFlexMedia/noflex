// SPDX-License-Identifier: AGPL-3.0-only
//
// Host-side wasmtime bindings for the NoFlex plugin WIT worlds.
//
// `metadata-plugin` is the simplest end-to-end happy path (single
// `metadata` export). `metadata-image-plugin` is the common TMDb/TVDB
// pattern (metadata + image). `notifier-plugin` and `user-link-plugin`
// cover the event-consumer and account-linking surfaces, and
// `post-scan-plugin` the scan-following surface.
// `embedder-plugin` serves local embedding providers; it is also the
// only convenience world importing `host-assets`. `ratings-plugin` serves
// providers that only add ratings. `scrobbler-plugin` pairs the scrobble
// event stream with `user-link` (the Trakt pattern), and `subtitle-plugin`
// serves online subtitle providers. The host attempts every world at
// instantiation and keeps whichever bindings succeed, so a component
// exporting any hook subset is served.
//
// Each `bindgen!` emits a `plugin` module; wrapping them in inline
// modules avoids the name collision on the shared host imports.

pub mod metadata_only {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "metadata-plugin",
        imports: { default: trappable },
    });
}

pub mod metadata_and_image {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "metadata-image-plugin",
        imports: { default: trappable },
    });
}

pub mod notifier {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "notifier-plugin",
        imports: { default: trappable },
    });
}

pub mod user_link {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "user-link-plugin",
        imports: { default: trappable },
    });
}

pub mod external_auth {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "external-auth-plugin",
        imports: { default: trappable },
    });
}

pub mod post_scan {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "post-scan-plugin",
        imports: { default: trappable },
    });
}

pub mod embedder {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "embedder-plugin",
        imports: { default: trappable },
    });
}

pub mod ratings {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "ratings-plugin",
        imports: { default: trappable },
    });
}

pub mod scrobbler {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "scrobbler-plugin",
        imports: { default: trappable },
    });
}

/// The imports through which plugins write library data they own. No
/// convenience world carries them, so this world exists only to generate
/// their host traits; guests import them beside a world of their own.
pub mod library_writes {
    wasmtime::component::bindgen!({
        inline: "
            package noflex:host-writes;

            world library-writes {
                import noflex:plugin/host-tags@0.2.0;
                import noflex:plugin/host-collections@0.2.0;
                import noflex:plugin/host-user-state@0.2.0;
            }
        ",
        path: "../noflex-plugin-wit/wit",
        world: "noflex:host-writes/library-writes",
        imports: { default: trappable },
    });
}

pub mod subtitle {
    wasmtime::component::bindgen!({
        path: "../noflex-plugin-wit/wit",
        world: "subtitle-plugin",
        imports: { default: trappable },
    });
}
