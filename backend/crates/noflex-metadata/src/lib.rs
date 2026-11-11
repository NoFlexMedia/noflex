// SPDX-License-Identifier: AGPL-3.0-only
//
// Shared metadata helpers: the pure domain logic for applying a provider
// `MetadataPayload` to a `Work`. The providers themselves ship as `.wasm`
// plugins (see `backend/plugins/`); this crate is deliberately free of
// the plugin host and GraphQL so the apply rules stay testable in
// isolation.

use noflex_core::{ExternalId, MetadataStatus, WorkField, providers::MetadataPayload};
use noflex_storage::domain::{CATEGORY_TAG_NAMESPACE, Work};

/// One tag the payload implies, in the unified taxonomy.
pub struct TagDraft {
    pub namespace: &'static str,
    pub display: String,
    /// Stored on the tag itself, so only what holds wherever it's used: a
    /// person's provider ids, under [`PERSON_IDS_KEY`].
    pub metadata: serde_json::Value,
    /// Stored on this work's assignment of the tag: a person's billing `order`
    /// and `credits`, each with a `role` and `character` when known. Empty
    /// otherwise.
    pub assignment: serde_json::Value,
}

/// Where a person tag's metadata keeps the providers' ids for them, as a list
/// of [`ExternalId`]s.
pub const PERSON_IDS_KEY: &str = "external_ids";

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
        // These classify the work the way genres do, so their lock covers
        // them too.
        if is_anime(payload) {
            push_draft(
                &mut tags,
                CATEGORY_TAG_NAMESPACE,
                "Anime".to_string(),
                serde_json::json!({}),
            );
        }
        for keyword in &payload.keywords {
            push_draft(&mut tags, "keyword", keyword.clone(), serde_json::json!({}));
        }
        if let Some(budget) = payload.budget_usd {
            push_draft(
                &mut tags,
                "scale",
                budget_scale(budget).to_string(),
                serde_json::json!({}),
            );
        }
    }
    if !locked(WorkField::Studios) {
        for studio in &payload.studios {
            push_draft(&mut tags, "studio", studio.clone(), serde_json::json!({}));
        }
    }
    if !locked(WorkField::People) {
        for person in &payload.people {
            let mut credit = serde_json::Map::new();
            if let Some(role) = &person.role {
                credit.insert("role".into(), role.clone().into());
            }
            if let Some(character) = &person.character {
                credit.insert("character".into(), character.clone().into());
            }
            // Someone who directed and starred is listed twice, one tag either way.
            let existing = tags
                .iter_mut()
                .find(|draft| draft.namespace == "person" && draft.display == person.name);
            if let Some(draft) = existing {
                if let Some(credits) = draft.assignment["credits"].as_array_mut() {
                    credits.push(credit.into());
                }
                let ids = serde_json::json!({ PERSON_IDS_KEY: person.external_ids });
                if !person.external_ids.is_empty()
                    && let Some(merged) = merge_tag_metadata(&draft.metadata, &ids)
                {
                    draft.metadata = merged;
                }
                continue;
            }
            let order = tags
                .iter()
                .filter(|draft| draft.namespace == "person")
                .count();
            tags.push(TagDraft {
                namespace: "person",
                display: person.name.clone(),
                metadata: if person.external_ids.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::json!({ PERSON_IDS_KEY: person.external_ids })
                },
                assignment: serde_json::json!({ "order": order, "credits": [credit] }),
            });
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

/// The provider ids a person tag's `metadata` keeps under
/// [`PERSON_IDS_KEY`]; empty when it has none or they don't parse.
pub fn person_ids(metadata: &serde_json::Value) -> Vec<ExternalId> {
    metadata
        .get(PERSON_IDS_KEY)
        .and_then(|ids| serde_json::from_value(ids.clone()).ok())
        .unwrap_or_default()
}

/// A tag's stored `metadata` with a new draft's laid over it, as a later
/// payload describes the same tag: the draft's keys win, except that the
/// person ids of both are kept, as each provider knows the person by its
/// own. `None` when nothing would change.
pub fn merge_tag_metadata(
    existing: &serde_json::Value,
    incoming: &serde_json::Value,
) -> Option<serde_json::Value> {
    let serde_json::Value::Object(incoming) = incoming else {
        return None;
    };
    let mut merged = match existing {
        serde_json::Value::Object(existing) => existing.clone(),
        _ => serde_json::Map::new(),
    };
    for (key, value) in incoming {
        if key == PERSON_IDS_KEY {
            let mut ids = person_ids(existing);
            for id in person_ids(&serde_json::json!({ PERSON_IDS_KEY: value })) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            merged.insert(key.clone(), serde_json::json!(ids));
        } else {
            merged.insert(key.clone(), value.clone());
        }
    }
    let merged = serde_json::Value::Object(merged);
    (&merged != existing).then_some(merged)
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

/// The production scale a budget in US dollars suggests. Nominal dollars, so
/// older films land a bucket low; this only has to group similar titles.
fn budget_scale(usd: u64) -> &'static str {
    match usd {
        0..5_000_000 => "Independent",
        5_000_000..40_000_000 => "Mid-budget",
        40_000_000..120_000_000 => "Big-budget",
        _ => "Blockbuster",
    }
}

/// Animation from Japan, by origin country or, when a provider gives none,
/// original language. Lives here rather than in providers so all of them
/// classify alike.
fn is_anime(payload: &MetadataPayload) -> bool {
    let animated = payload.genres.iter().any(|genre| {
        genre.eq_ignore_ascii_case("animation") || genre.eq_ignore_ascii_case("anime")
    });
    let japanese = if payload.origin_countries.is_empty() {
        payload
            .original_language
            .as_deref()
            .is_some_and(|language| language.eq_ignore_ascii_case("ja"))
    } else {
        payload
            .origin_countries
            .iter()
            .any(|country| country.eq_ignore_ascii_case("JP"))
    };
    animated && japanese
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
        assignment: serde_json::json!({}),
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
            metadata_provider: None,
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
            original_language: None,
            origin_countries: vec![],
            keywords: vec![],
            budget_usd: None,
        }
    }

    #[test]
    fn keywords_and_budget_become_tags_under_the_genre_lock() {
        let mut payload = empty_payload();
        payload.keywords = vec!["heist".into()];
        payload.budget_usd = Some(150_000_000);
        let mut unlocked = work();
        let tags = apply_payload(&mut unlocked, &payload).tags;
        assert!(
            tags.iter()
                .any(|t| t.namespace == "keyword" && t.display == "heist")
        );
        assert!(
            tags.iter()
                .any(|t| t.namespace == "scale" && t.display == "Blockbuster")
        );

        let mut locked = work();
        locked.locked_fields = vec![WorkField::Genres];
        assert!(apply_payload(&mut locked, &payload).tags.is_empty());
    }

    #[test]
    fn anime_is_japanese_animation() {
        let category_tags = |genres: &[&str], language: Option<&str>, countries: &[&str]| {
            let mut payload = empty_payload();
            payload.genres = genres.iter().map(|g| g.to_string()).collect();
            payload.original_language = language.map(str::to_string);
            payload.origin_countries = countries.iter().map(|c| c.to_string()).collect();
            let mut work = work();
            apply_payload(&mut work, &payload)
                .tags
                .iter()
                .filter(|tag| tag.namespace == CATEGORY_TAG_NAMESPACE)
                .count()
        };
        assert_eq!(category_tags(&["Animation"], Some("ja"), &["JP"]), 1);
        assert_eq!(category_tags(&["Animation"], Some("ja"), &[]), 1);
        assert_eq!(
            category_tags(&["Animation"], Some("en"), &["US"]),
            0,
            "Western animation"
        );
        assert_eq!(
            category_tags(&["Drama"], Some("ja"), &["JP"]),
            0,
            "Japanese live action"
        );
        assert_eq!(
            category_tags(&["Animation"], Some("ja"), &["US"]),
            0,
            "a stated country wins over the language"
        );
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
                external_ids: vec![],
            },
            MetadataPerson {
                name: "Actor Two".into(),
                role: Some("Actor".into()),
                character: Some("Protagonist".into()),
                external_ids: vec![],
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
    fn a_later_payload_adds_its_person_ids_to_the_tags() {
        let tmdb = ExternalId {
            kind: ExternalIdKind::Tmdb,
            value: "person/1".into(),
        };
        let tvdb = ExternalId {
            kind: ExternalIdKind::Tvdb,
            value: "people/9".into(),
        };
        let stored = serde_json::json!({ PERSON_IDS_KEY: [tmdb] });
        let merged = merge_tag_metadata(&stored, &serde_json::json!({ PERSON_IDS_KEY: [tvdb] }))
            .expect("a new id changes the tag");
        assert_eq!(person_ids(&merged), vec![tmdb.clone(), tvdb]);
        assert_eq!(
            merge_tag_metadata(&merged, &serde_json::json!({ PERSON_IDS_KEY: [tmdb] })),
            None,
            "an id it has already changes nothing"
        );
        assert_eq!(merge_tag_metadata(&merged, &serde_json::json!({})), None);
        assert_eq!(
            merge_tag_metadata(&serde_json::json!({}), &stored),
            Some(stored),
            "a tag made before it knew any id learns them"
        );
    }

    #[test]
    fn people_keep_billing_order_and_every_part_on_the_assignment() {
        let mut work = work();
        let mut payload = empty_payload();
        payload.people = vec![
            MetadataPerson {
                name: "Director One".into(),
                role: Some("Director".into()),
                character: None,
                external_ids: vec![],
            },
            MetadataPerson {
                name: "Actor Two".into(),
                role: None,
                character: Some("Protagonist".into()),
                external_ids: vec![ExternalId {
                    kind: ExternalIdKind::Tmdb,
                    value: "person/2".into(),
                }],
            },
            MetadataPerson {
                name: "Director One".into(),
                role: Some("Actor".into()),
                character: Some("Cameo".into()),
                external_ids: vec![],
            },
        ];

        let report = apply_payload(&mut work, &payload);

        assert_eq!(report.tags.len(), 2);
        assert_eq!(report.tags[0].metadata, serde_json::json!({}));
        assert_eq!(
            report.tags[0].assignment,
            serde_json::json!({
                "order": 0,
                "credits": [
                    { "role": "Director" },
                    { "role": "Actor", "character": "Cameo" },
                ],
            })
        );
        assert_eq!(
            report.tags[1].metadata,
            serde_json::json!({ "external_ids": [{ "kind": "Tmdb", "value": "person/2" }] })
        );
        assert_eq!(
            report.tags[1].assignment,
            serde_json::json!({ "order": 1, "credits": [{ "character": "Protagonist" }] })
        );
    }
}
