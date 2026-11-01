// SPDX-License-Identifier: AGPL-3.0-only
//
// Client-side request pacing, shared shape with the MusicBrainz plugin.
// TMDb answers 429 past roughly 50 requests a second per IP, so the guest
// spaces its own API requests rather than burning calls on rejections.
// Pure (marks in, marks out) so it is testable off-target.

use std::cell::Cell;

const NANOS_PER_SEC: u64 = 1_000_000_000;

/// Hands out send slots at least `interval_ns` apart, shared by every
/// concurrent call on the (single-threaded) guest instance.
pub struct Pacer {
    interval_ns: u64,
    max_wait_ns: u64,
    next_ns: Cell<u64>,
}

impl Pacer {
    /// `max_wait_ns` bounds how far ahead a caller may queue; past it the
    /// caller is told to retry instead, so a burst cannot push a call past
    /// its host deadline while it waits for a slot.
    pub const fn new(interval_ns: u64, max_wait_ns: u64) -> Self {
        Self {
            interval_ns,
            max_wait_ns,
            next_ns: Cell::new(0),
        }
    }

    /// Reserve the next slot at or after `now_ns`, returning the mark to
    /// send at. `Err(retry_after_secs)` when the next free slot is more than
    /// `max_wait_ns` away; nothing is reserved then.
    pub fn reserve(&self, now_ns: u64) -> Result<u64, u32> {
        let slot = self.next_ns.get().max(now_ns);
        let wait = slot - now_ns;
        if wait > self.max_wait_ns {
            return Err(wait.div_ceil(NANOS_PER_SEC).try_into().unwrap_or(u32::MAX));
        }
        self.next_ns.set(slot + self.interval_ns);
        Ok(slot)
    }

    /// The upstream said to back off for `secs`: no slot before then.
    pub fn back_off(&self, now_ns: u64, secs: u32) {
        let until = now_ns.saturating_add(u64::from(secs) * NANOS_PER_SEC);
        self.next_ns.set(self.next_ns.get().max(until));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = NANOS_PER_SEC;

    #[test]
    fn concurrent_reservations_are_spaced_one_interval_apart() {
        let pacer = Pacer::new(S, 5 * S);
        assert_eq!(pacer.reserve(10 * S), Ok(10 * S));
        assert_eq!(pacer.reserve(10 * S), Ok(11 * S));
        assert_eq!(pacer.reserve(10 * S + 1), Ok(12 * S));
        // After an idle gap the next slot is immediate again.
        assert_eq!(pacer.reserve(100 * S), Ok(100 * S));
    }

    #[test]
    fn a_long_queue_answers_retry_after_instead_of_waiting() {
        let pacer = Pacer::new(S, 2 * S);
        for _ in 0..3 {
            pacer.reserve(0).unwrap();
        }
        assert_eq!(pacer.reserve(0), Err(3));
        // The refusal reserved nothing.
        assert_eq!(pacer.reserve(S), Ok(3 * S));
    }

    #[test]
    fn upstream_backoff_pushes_the_next_slot() {
        let pacer = Pacer::new(S, 10 * S);
        pacer.back_off(0, 4);
        assert_eq!(pacer.reserve(0), Ok(4 * S));
    }
}
