// SPDX-License-Identifier: AGPL-3.0-only
//
// Typed newtype wrappers around UUIDv7. UUIDv7 is time-ordered, so they
// sort by creation and give backends good index locality. Wire format is
// the raw 16 bytes; backends choose `BLOB` / `UUID` / `BINARY(16)` columns.
// On GraphQL every typed id is the built-in `ID` scalar (hyphenated UUID
// text), so clients see one identifier type and resolvers take the newtype
// directly: malformed input fails argument coercion before a resolver runs.

use async_graphql::NewType;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! typed_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, NewType)]
        #[graphql(name = "ID")]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub fn from_uuid(u: Uuid) -> Self {
                Self(u)
            }

            pub fn into_uuid(self) -> Uuid {
                self.0
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

typed_id!(
    WorkId,
    "Identifier for a logical work (movie, episode, track, ...)."
);
typed_id!(
    EditionId,
    "Identifier for an edition of a work (Theatrical, Director's Cut, ...)."
);
typed_id!(MediaFileId, "Identifier for a physical media file on disk.");
typed_id!(UserId, "Identifier for a user account.");
typed_id!(
    InviteId,
    "Identifier for an invitation to create an account."
);
typed_id!(
    LinkedServerId,
    "Identifier for a user's link to another NoFlex server."
);
typed_id!(
    DeviceId,
    "Identifier for a registered client device, chosen by the client so it survives sign-outs."
);
typed_id!(
    OptimizedVersionId,
    "Identifier for a prepared copy of a media file, derived from it and never a media file itself."
);
typed_id!(
    DownloadedSubtitleId,
    "Identifier for a subtitle fetched from a provider for one media file."
);
typed_id!(
    LibraryId,
    "Identifier for a library (a configured root of media)."
);
typed_id!(SessionId, "Identifier for an authenticated session.");
typed_id!(TagId, "Identifier for a tag.");
typed_id!(
    ExportJobId,
    "Identifier for an export job (a user-requested offline download)."
);
typed_id!(
    CollectionId,
    "Identifier for a curated collection of works within a library."
);
typed_id!(PlaylistId, "Identifier for a user-owned ordered playlist.");
typed_id!(
    ApiKeyId,
    "Identifier for an API key (a long-lived programmatic credential)."
);
typed_id!(
    DeviceCodeId,
    "Identifier for a device-linking code request."
);
typed_id!(WatchRoomId, "Identifier for a synchronized watch room.");
typed_id!(
    PlaySessionId,
    "Identifier for one viewing in the play history."
);
typed_id!(
    LiveTvSourceId,
    "Identifier for a Live TV playlist and guide source."
);
typed_id!(ChannelId, "Identifier for a Live TV channel.");
typed_id!(
    EpgProgramId,
    "Identifier for an electronic programme guide entry."
);
typed_id!(
    RecordingScheduleId,
    "Identifier for a DVR recording schedule."
);
typed_id!(RecordingId, "Identifier for a DVR recording.");
