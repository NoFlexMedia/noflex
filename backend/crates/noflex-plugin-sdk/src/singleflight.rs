// SPDX-License-Identifier: AGPL-3.0-only
//
// Request coalescing helper. Single-flight semantics require a notify
// primitive that lets N waiters block on one driver's result; tokio's
// `Notify` is the natural fit, but the SDK avoids a runtime dep so it
// works inside WASI 0.3 components that don't link tokio.
//
// v1 ships a **sequential** coalescer — useful for plugins that issue
// upstream calls one at a time and want a small "remember the last
// answer for this key" cache. True concurrent singleflight (N parallel
// callers, one upstream call, all return the same value) lands when
// the WASI 0.3 future / channel primitives stabilize.
//
// For aggressive parallel deduplication today, use [`cache::TtlCache`]
// with a short TTL — the second concurrent caller picks up the cache
// entry the first writes.

use std::{collections::HashMap, hash::Hash, sync::Mutex};

/// Stores the most recent successful result per key. Calls to
/// [`SingleFlight::cached`] return the cached value if present; the
/// caller is expected to record fresh results via [`SingleFlight::store`].
///
/// Single-threaded interior mutability — wrap in `tokio::sync::Mutex` if
/// you need cross-await access (you generally don't; plugins are
/// single-instance under WASI 0.3 and their async runtime is the host's).
pub struct SingleFlight<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    cache: Mutex<HashMap<K, V>>,
}

impl<K, V> SingleFlight<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Look up the cached value for `key`, if any.
    pub fn cached(&self, key: &K) -> Option<V> {
        self.cache.lock().expect("poisoned").get(key).cloned()
    }

    /// Record a successful result for `key`.
    pub fn store(&self, key: K, value: V) {
        self.cache.lock().expect("poisoned").insert(key, value);
    }

    /// Drop the entry for `key` (useful when an upstream failure should
    /// not be remembered as "the result").
    pub fn forget(&self, key: &K) {
        self.cache.lock().expect("poisoned").remove(key);
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.cache.lock().expect("poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<K, V> Default for SingleFlight<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_then_cached_returns_value() {
        let sf: SingleFlight<&str, i32> = SingleFlight::new();
        sf.store("a", 1);
        assert_eq!(sf.cached(&"a"), Some(1));
    }

    #[test]
    fn forget_drops_entry() {
        let sf: SingleFlight<&str, i32> = SingleFlight::new();
        sf.store("a", 1);
        sf.forget(&"a");
        assert_eq!(sf.cached(&"a"), None);
    }
}
