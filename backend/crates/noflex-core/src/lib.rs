// SPDX-License-Identifier: AGPL-3.0-only
//
// noflex-core defines the domain enums, IDs, errors, and the **plugin
// extension trait surface** — the seam that every metadata provider,
// auth backend, scrobbler, etc. plugs into. Trait signatures here are
// semver-stable (see CONTRIBUTING.md).

pub mod bundle;
pub mod error;
pub mod events;
pub mod ids;
pub mod model;
pub mod providers;
pub mod system_dirs;
pub mod text;

pub use error::{PluginError, PluginResult};
pub use events::{DomainEvent, DomainEventBus};
pub use ids::{
    ApiKeyId, ChannelId, CollectionId, DeviceCodeId, DeviceId, EditionId, EpgProgramId,
    ExportJobId, InviteId, LibraryId, LinkedServerId, LiveTvSourceId, MediaFileId,
    OptimizedVersionId, PlaySessionId, PlaylistId, RecordingId, RecordingScheduleId, SessionId,
    TagId, UserId, WatchRoomId, WorkId,
};
pub use model::{
    AudioCodec, AudioObjectType, ChannelLayout, ColorPrimaries, DolbyVisionProfile,
    EmbeddingTarget, ExternalId, ExternalIdKind, HdrTransfer, ImageKind, MatrixCoefficients,
    MetadataStatus, PixelFormat, PlayTarget, PlayTargetKind, SubtitleCodec, TagTarget,
    TonemapCurve, UserRole, VideoCodec, VideoProfile, WorkField, WorkKind,
};
pub use text::slugify;
