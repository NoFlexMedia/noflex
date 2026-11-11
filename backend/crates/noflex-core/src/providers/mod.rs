// SPDX-License-Identifier: AGPL-3.0-only
//
// Plugin extension trait surface. Built-in implementations and WASM
// components plug in through these same traits. Each hook interface in the
// canonical WIT (`noflex-plugin-wit`) has a mirror trait here with owned,
// serializable arguments; the WIT wins when the two disagree. `background`
// is the one exemption: it is host-scheduler plumbing (manifest-declared
// tasks, opaque cursors), not a provider capability, so it has no mirror.
//
// Stability contract: every trait here is semver-stable. Method
// additions must be defaulted. Breaking changes ship as `FooProvider2` with
// a two-minor-version deprecation window for the old version.

pub mod auth;
pub mod embedder;
pub mod external_auth;
pub mod image;
pub mod metadata;
pub mod notifier;
pub mod post_scan;
pub mod ratings;
pub mod resolver;
pub mod scrobbler;
pub mod search;
pub mod subtitle;
pub mod user_link;

pub use auth::{AuthCredentials, AuthProvider, AuthVerdict};
pub use embedder::{EmbedRequest, EmbedResult, Embedder};
pub use external_auth::{ExternalAuthProvider, VerifiedIdentity};
pub use image::{ImageCandidate, ImageProvider};
pub use metadata::{MetadataMatch, MetadataPayload, MetadataProvider, SearchQuery};
pub use notifier::{Notifier, NotifyEvent};
pub use post_scan::PostScanTask;
pub use ratings::{
    DEFAULT_RATING_SOURCES, Rating, RatingsProvider, is_source_slug, rating_display,
    rating_source_name,
};
pub use resolver::{ItemResolver, ResolvedItem};
pub use scrobbler::{ScrobbleAction, ScrobbleEvent, Scrobbler};
pub use search::{SearchHit, SearchProvider};
pub use subtitle::{SubtitleFile, SubtitleMatch, SubtitleProvider, SubtitleQuery};
pub use user_link::{LinkChallenge, UserLinkProvider, UserSecret};
