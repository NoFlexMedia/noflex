// SPDX-License-Identifier: AGPL-3.0-only
//
// What the server is still doing to works it has just found, so clients can
// say so instead of showing a card that looks finished and wrong. In memory:
// a restart runs a fresh sweep, which asks about everything unmatched again.
//
// `MetadataStatus::Unconfirmed` can't answer this. A work with no match stays
// unconfirmed for good, and nothing is happening to it.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use async_graphql::Enum;
use noflex_core::{LibraryId, WorkId};
use time::OffsetDateTime;

use crate::domain::Work;

/// How far along a newly found work is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum IndexingStage {
    /// Its library's scan, which found it, is still running.
    Adding,
    /// Waiting for, or in, the metadata sweep's search for a match.
    FindingDetails,
}

#[derive(Default)]
struct State {
    scans: HashMap<LibraryId, OffsetDateTime>,
    sweeps: u32,
    asked: HashSet<WorkId>,
}

/// Fed by scan events and the metadata worker, read by `Work.indexing`.
#[derive(Default)]
pub struct IndexingProgress {
    state: Mutex<State>,
}

impl IndexingProgress {
    /// `library` began a scan; what it adds from `now` on is being added.
    pub fn scan_started(&self, library: LibraryId, now: OffsetDateTime) {
        self.with(|state| {
            state.scans.insert(library, now);
        });
    }

    pub fn scan_finished(&self, library: LibraryId) {
        self.with(|state| {
            state.scans.remove(&library);
        });
    }

    /// Events were missed, so a finished scan may never be heard of. Better
    /// to claim nothing than to show a card as working forever.
    pub fn forget_scans(&self) {
        self.with(|state| state.scans.clear());
    }

    /// A metadata sweep is queued or running.
    pub fn sweep_begun(&self) {
        self.with(|state| state.sweeps += 1);
    }

    pub fn sweep_ended(&self) {
        self.with(|state| state.sweeps = state.sweeps.saturating_sub(1));
    }

    /// The sweep has had its answer on `work`, and found nothing to apply.
    /// Later sweeps try again, but as a retry rather than as new work.
    pub fn asked(&self, work: WorkId) {
        self.with(|state| {
            state.asked.insert(work);
        });
    }

    /// `None` once the server has done what it can, match or no match.
    pub fn stage(&self, work: &Work) -> Option<IndexingStage> {
        let state = self.state.lock().ok()?;
        let found_by_scan = state
            .scans
            .get(&work.library_id)
            .is_some_and(|started| work.added_at >= *started);
        if found_by_scan && work.awaits_identification() {
            Some(IndexingStage::Adding)
        } else if state.sweeps > 0
            && work.awaits_identification()
            && !state.asked.contains(&work.id)
        {
            Some(IndexingStage::FindingDetails)
        } else {
            None
        }
    }

    fn with(&self, change: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            change(&mut state);
        }
    }
}

#[cfg(test)]
mod tests {
    use noflex_core::{MetadataStatus, WorkKind};
    use time::Duration;

    use super::*;

    fn work(library: LibraryId, added_at: OffsetDateTime) -> Work {
        Work {
            id: WorkId::new(),
            kind: WorkKind::Movie,
            parent_id: None,
            library_id: library,
            title: "Film".into(),
            sort_title: "film".into(),
            year: None,
            release_date: None,
            overview: None,
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
            added_at,
            updated_at: added_at,
        }
    }

    #[test]
    fn a_rescan_only_claims_what_it_found() {
        let progress = IndexingProgress::default();
        let library = LibraryId::new();
        let started = OffsetDateTime::now_utc();
        let unmatched = work(library, started - Duration::days(30));
        let found = work(library, started + Duration::seconds(1));
        assert_eq!(progress.stage(&found), None);

        progress.scan_started(library, started);
        assert_eq!(progress.stage(&found), Some(IndexingStage::Adding));
        assert_eq!(progress.stage(&unmatched), None);

        progress.scan_finished(library);
        assert_eq!(progress.stage(&found), None);
    }

    #[test]
    fn a_work_with_no_match_stops_indexing_while_the_sweep_goes_on() {
        let progress = IndexingProgress::default();
        let now = OffsetDateTime::now_utc();
        let unmatched = work(LibraryId::new(), now);
        let mut confirmed = work(LibraryId::new(), now);
        confirmed.metadata_status = MetadataStatus::Confirmed;

        progress.sweep_begun();
        assert_eq!(
            progress.stage(&unmatched),
            Some(IndexingStage::FindingDetails)
        );
        assert_eq!(progress.stage(&confirmed), None);

        progress.asked(unmatched.id);
        assert_eq!(progress.stage(&unmatched), None);
    }
}
