// SPDX-License-Identifier: AGPL-3.0-only
//
// Tiny LRU cache helper plugins can drop into their internal state.
// Designed for the common pattern: cache the result of an upstream API
// call keyed by `(endpoint, external_id)` so subsequent
// `metadata::fetch` and `image::fetch` calls for the same id reuse data.

use std::{hash::Hash, num::NonZeroUsize, time::Duration};

use lru::LruCache;
use serde::{Deserialize, Serialize};

/// A bounded LRU with per-entry expiry. Not thread-safe — single-instance
/// plugins access it directly; multi-store plugins (rare in v1) wrap in a
/// `std::sync::Mutex` themselves.
pub struct TtlCache<K, V>
where
    K: Hash + Eq,
{
    inner: LruCache<K, Entry<V>>,
    default_ttl: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry<V> {
    inserted_secs: u64,
    ttl_secs: u64,
    value: V,
}

impl<K: Hash + Eq, V: Clone> TtlCache<K, V> {
    pub fn new(capacity: usize, default_ttl: Duration) -> Self {
        Self {
            inner: LruCache::new(NonZeroUsize::new(capacity.max(1)).unwrap()),
            default_ttl,
        }
    }

    /// Insert with the cache's default TTL.
    pub fn put(&mut self, key: K, value: V) {
        let now = now_secs();
        self.inner.put(
            key,
            Entry {
                inserted_secs: now,
                ttl_secs: self.default_ttl.as_secs().max(1),
                value,
            },
        );
    }

    /// Insert with an explicit TTL.
    pub fn put_with_ttl(&mut self, key: K, value: V, ttl: Duration) {
        let now = now_secs();
        self.inner.put(
            key,
            Entry {
                inserted_secs: now,
                ttl_secs: ttl.as_secs().max(1),
                value,
            },
        );
    }

    /// Retrieve a still-fresh entry. Expired entries are removed lazily.
    pub fn get(&mut self, key: &K) -> Option<V> {
        let now = now_secs();
        let entry = self.inner.peek(key)?;
        if now.saturating_sub(entry.inserted_secs) >= entry.ttl_secs {
            self.inner.pop(key);
            return None;
        }
        // peek doesn't bump recency; call get to do so.
        self.inner.get(key).map(|e| e.value.clone())
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

fn now_secs() -> u64 {
    // Inside a WASM plugin, `SystemTime::now` resolves via wasi:clocks.
    // On the host (where the SDK also compiles for testing), this is the
    // normal system time.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_then_get_returns_value() {
        let mut cache: TtlCache<&str, i32> = TtlCache::new(4, Duration::from_secs(60));
        cache.put("a", 1);
        assert_eq!(cache.get(&"a"), Some(1));
    }

    #[test]
    fn lru_eviction_when_capacity_exceeded() {
        let mut cache: TtlCache<i32, i32> = TtlCache::new(2, Duration::from_secs(60));
        cache.put(1, 11);
        cache.put(2, 22);
        cache.put(3, 33); // evicts 1
        assert_eq!(cache.get(&1), None);
        assert_eq!(cache.get(&2), Some(22));
        assert_eq!(cache.get(&3), Some(33));
    }

    #[test]
    fn expired_entries_are_dropped() {
        // ttl_secs floors to 1, so we set up an immediately-expired entry by
        // back-dating the inserted time. Use the put_with_ttl path with a
        // 1s TTL and sleep just over a second.
        let mut cache: TtlCache<&str, i32> = TtlCache::new(4, Duration::from_secs(60));
        cache.put_with_ttl("a", 1, Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(cache.get(&"a"), None);
    }
}
