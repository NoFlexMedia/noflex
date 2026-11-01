// SPDX-License-Identifier: AGPL-3.0-only
//
// Shared metadata helpers: the pure domain logic for applying a provider
// `MetadataPayload` to a `Work`. The providers themselves ship as `.wasm`
// plugins (see `backend/plugins/`); this crate is deliberately free of
// the plugin host and GraphQL so the apply rules stay testable in
// isolation.

use noflex_core::{ExternalId, MetadataStatus, WorkField, providers::MetadataPayload};
use noflex_storage::domain::Work;

/// One tag the payload implies, in the unified taxonomy.
pub struct TagDraft {
    pub namespace: &'static str,
    pub display: String,
    /// Role/character/photo_url for people; empty object otherwise.
    pub metadata: serde_json::Value,
}

pub struct ApplyReport {
    /// Fields actually overwritten (unlocked + payload had a value).
    pub changed: Vec<WorkField>,
    pub tags: Vec<TagDraft>,
}

/// Apply a provider payload to a work. Per-field locks always win: a
/// locked field is never overwritten and never produces tag drafts.
/// `None`/empty payload values never clear an existing value.
/// Escalates `metadata_status` Unconfirmed -> Confirmed; Confirmed and
/// UserEdited are left alone. Does not touch `updated_at` (caller's job).
pub fn apply_payload(work: &mut Work, payload: &MetadataPayload) -> ApplyReport {
    // Snapshot the locks up front: the closure would otherwise hold an
    // immutable borrow of `work` across the mutable writes below.
    let locks = work.locked_fields.clone();
    let locked = |field: WorkField| locks.contains(&field);
    let mut changed = Vec::new();

    if !locked(WorkField::Title) && !payload.title.trim().is_empty() {
        work.title = payload.title.clone();
        changed.push(WorkField::Title);
    }
    if !locked(WorkField::SortTitle)
        && let Some(sort_title) = &payload.sort_title
    {
        work.sort_title = sort_title.clone();
        changed.push(WorkField::SortTitle);
    }
    if !locked(WorkField::OriginalTitle)
        && let Some(original_title) = &payload.original_title
    {
        work.original_title = Some(original_title.clone());
        changed.push(WorkField::OriginalTitle);
    }
    if !locked(WorkField::Year)
        && let Some(year) = payload.year
    {
        work.year = Some(year);
        changed.push(WorkField::Year);
    }
    if !locked(WorkField::ReleaseDate)
        && let Some(release_date) = &payload.release_date
    {
        work.release_date = Some(release_date.clone());
        changed.push(WorkField::ReleaseDate);
    }
    if !locked(WorkField::Overview)
        && let Some(overview) = &payload.overview
    {
        work.overview = Some(overview.clone());
        changed.push(WorkField::Overview);
    }
    if !locked(WorkField::Tagline)
        && let Some(tagline) = &payload.tagline
    {
        work.tagline = Some(tagline.clone());
        changed.push(WorkField::Tagline);
    }
    if !locked(WorkField::CommunityRating)
        && let Some(community_rating) = payload.community_rating
    {
        work.community_rating = Some(community_rating);
        changed.push(WorkField::CommunityRating);
    }

    // External ids are never lockable: always merged, never cleared.
    for id in &payload.external_ids {
        merge_external_id(work, id.clone());
    }

    let mut tags = Vec::new();
    if !locked(WorkField::Genres) {
        for genre in &payload.genres {
            push_draft(&mut tags, "genre", genre.clone(), serde_json::json!({}));
        }
    }
    if !locked(WorkField::Studios) {
        for studio in &payload.studios {
            push_draft(&mut tags, "studio", studio.clone(), serde_json::json!({}));
        }
    }
    if !locked(WorkField::People) {
        for person in &payload.people {
            let mut metadata = serde_json::Map::new();
            if let Some(role) = &person.role {
                metadata.insert("role".into(), role.clone().into());
            }
            if let Some(character) = &person.character {
                metadata.insert("character".into(), character.clone().into());
            }
            if let Some(photo_url) = &person.photo_url {
                metadata.insert("photo_url".into(), photo_url.clone().into());
            }
            push_draft(
                &mut tags,
                "person",
                person.name.clone(),
                serde_json::Value::Object(metadata),
            );
        }
    }
    if !locked(WorkField::ParentalRating)
        && let Some(rating) = &payload.rating
        && !rating.trim().is_empty()
    {
        push_draft(&mut tags, "parental", rating.clone(), serde_json::json!({}));
    }

    if work.metadata_status == MetadataStatus::Unconfirmed {
        work.metadata_status = MetadataStatus::Confirmed;
    }

    ApplyReport { changed, tags }
}

/// Add `id` to the work's external ids unless an id with the same
/// (kind, value) is already there. Returns `true` when it was added.
pub fn merge_external_id(work: &mut Work, id: ExternalId) -> bool {
    if work
        .external_ids
        .iter()
        .any(|e| e.kind == id.kind && e.value == id.value)
    {
        return false;
    }
    work.external_ids.push(id);
    true
}

/// First draft per (namespace, display) wins — providers occasionally
/// repeat a name within one list.
fn push_draft(
    drafts: &mut Vec<TagDraft>,
    namespace: &'static str,
    display: String,
    metadata: serde_json::Value,
) {
    if drafts
        .iter()
        .any(|d| d.namespace == namespace && d.display == display)
    {
        return;
    }
    drafts.push(TagDraft {
        namespace,
        display,
        metadata,
    });
}

#[cfg(test)]
mod tests {
    use noflex_core::{
        ExternalIdKind, LibraryId, WorkId, WorkKind, providers::metadata::MetadataPerson,
    };
    use time::OffsetDateTime;

    use super::*;

    fn work() -> Work {
        let now = OffsetDateTime::now_utc();
        Work {
            id: WorkId::new(),
            kind: WorkKind::Movie,
            parent_id: None,
            library_id: LibraryId::new(),
            title: "fs-title".into(),
            sort_title: "fs-title".into(),
            year: None,
            release_date: None,
            overview: Some("existing overview".into()),
            tagline: None,
            original_title: None,
            community_rating: None,
            external_ids: vec![],
            index_number: None,
            index_end: None,
            parent_index: None,
            metadata_status: MetadataStatus::Unconfirmed,
            locked_fields: vec![],
            added_at: now,
            updated_at: now,
        }
    }

    fn empty_payload() -> MetadataPayload {
        MetadataPayload {
            external_ids: vec![],
            title: "Provider Title".into(),
            sort_title: None,
            original_title: None,
            year: None,
            release_date: None,
            overview: None,
            tagline: None,
            runtime_ms: None,
            genres: vec![],
            studios: vec![],
            people: vec![],
            images: vec![],
            rating: None,
            community_rating: None,
        }
    }

    #[test]
    fn locked_title_survives_while_overview_applies() {
        let mut work = work();
        work.locked_fields = vec![WorkField::Title];
        let mut payload = empty_payload();
        payload.overview = Some("provider overview".into());

        let report = apply_payload(&mut work, &payload);

        assert_eq!(work.title, "fs-title");
        assert_eq!(work.overview.as_deref(), Some("provider overview"));
        assert_eq!(report.changed, vec![WorkField::Overview]);
    }

    #[test]
    fn none_overview_does_not_clear_existing() {
        let mut work = work();
        let report = apply_payload(&mut work, &empty_payload());
        assert_eq!(work.overview.as_deref(), Some("existing overview"));
        assert!(report.changed.contains(&WorkField::Title));
        assert!(!report.changed.contains(&WorkField::Overview));
    }

    #[test]
    fn status_escalates_only_up_the_ladder() {
        let mut work = work();
        apply_payload(&mut work, &empty_payload());
        assert_eq!(work.metadata_status, MetadataStatus::Confirmed);

        // Confirmed stays Confirmed on a later refresh.
        apply_payload(&mut work, &empty_payload());
        assert_eq!(work.metadata_status, MetadataStatus::Confirmed);

        // UserEdited is sticky.
        work.metadata_status = MetadataStatus::UserEdited;
        apply_payload(&mut work, &empty_payload());
        assert_eq!(work.metadata_status, MetadataStatus::UserEdited);
    }

    #[test]
    fn external_ids_merge_without_duplicates() {
        let mut work = work();
        work.external_ids = vec![
            ExternalId {
                kind: ExternalIdKind::Imdb,
                value: "tt1".into(),
            },
            ExternalId {
                kind: ExternalIdKind::Tmdb,
                value: "42".into(),
            },
        ];
        let mut payload = empty_payload();
        payload.external_ids = vec![
            // Same kind + value: dropped.
            ExternalId {
                kind: ExternalIdKind::Tmdb,
                value: "42".into(),
            },
            // Same value, different kind: kept.
            ExternalId {
                kind: ExternalIdKind::Tvdb,
                value: "42".into(),
            },
        ];
        apply_payload(&mut work, &payload);
        let pairs: Vec<_> = work
            .external_ids
            .iter()
            .map(|e| (e.kind, e.value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                (ExternalIdKind::Imdb, "tt1"),
                (ExternalIdKind::Tmdb, "42"),
                (ExternalIdKind::Tvdb, "42"),
            ]
        );

        let new = ExternalId {
            kind: ExternalIdKind::AniDb,
            value: "7".into(),
        };
        assert!(merge_external_id(&mut work, new.clone()));
        assert!(!merge_external_id(&mut work, new));
    }

    #[test]
    fn tag_drafts_respect_locks_and_carry_only_present_person_keys() {
        let mut work = work();
        work.locked_fields = vec![WorkField::People, WorkField::ParentalRating];
        let mut payload = empty_payload();
        payload.genres = vec!["Drama".into(), "Drama".into()];
        payload.studios = vec!["A24".into()];
        payload.rating = Some("R".into());
        payload.people = vec![
            MetadataPerson {
                name: "Director One".into(),
                role: Some("Director".into()),
                character: None,
                photo_url: None,
            },
            MetadataPerson {
                name: "Actor Two".into(),
                role: Some("Actor".into()),
                character: Some("Protagonist".into()),
                photo_url: Some("https://img/p.jpg".into()),
            },
        ];

        let report = apply_payload(&mut work, &payload);

        // Locked gates produce no drafts at all.
        assert!(report.tags.iter().all(|t| t.namespace != "person"));
        assert!(report.tags.iter().all(|t| t.namespace != "parental"));
        // In-namespace display dedupe.
        let genres: Vec<_> = report
            .tags
            .iter()
            .filter(|t| t.namespace == "genre")
            .collect();
        assert_eq!(genres.len(), 1);
        assert_eq!(genres[0].display, "Drama");
        assert_eq!(genres[0].metadata, serde_json::json!({}));
        assert!(
            report
                .tags
                .iter()
                .any(|t| t.namespace == "studio" && t.display == "A24")
        );
    }

    #[test]
    fn person_metadata_carries_only_present_keys() {
        let mut work = work();
        let mut payload = empty_payload();
        payload.people = vec![
            MetadataPerson {
                name: "Director One".into(),
                role: Some("Director".into()),
                character: None,
                photo_url: None,
            },
            MetadataPerson {
                name: "Actor Two".into(),
                role: None,
                character: Some("Protagonist".into()),
                photo_url: Some("https://img/p.jpg".into()),
            },
        ];

        let report = apply_payload(&mut work, &payload);

        assert_eq!(report.tags.len(), 2);
        assert_eq!(
            report.tags[0].metadata,
            serde_json::json!({ "role": "Director" })
        );
        assert_eq!(
            report.tags[1].metadata,
            serde_json::json!({
                "character": "Protagonist",
                "photo_url": "https://img/p.jpg",
            })
        );
    }
}
