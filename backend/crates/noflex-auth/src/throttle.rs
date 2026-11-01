// SPDX-License-Identifier: AGPL-3.0-only

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Remote address of the HTTP client, published into request data by
/// the transport so credential checks can be throttled per client as
/// well as per account. Absent → throttling is per account only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientAddr(pub IpAddr);

/// Failures tolerated inside one [`WINDOW`] before lockouts start.
const FREE_FAILURES: u32 = 5;
/// A key's failure count resets once this long passes without a failure.
const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Ceiling on a single lockout; doubles from one second up to this.
const MAX_LOCKOUT: Duration = Duration::from_secs(15 * 60);
/// Bound on tracked keys, so a spray of distinct usernames or addresses
/// can't grow the map without limit.
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Account(String),
    Client(IpAddr),
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    failures: u32,
    last_failure: Instant,
    locked_until: Option<Instant>,
}

impl Entry {
    fn expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.last_failure) >= WINDOW
            && self.locked_until.is_none_or(|until| until <= now)
    }
}

/// In-process brute-force guard for password checks, keyed by account
/// name (case-insensitive) and, when known, client address.
///
/// The first five failures per key inside a 15-minute window are free;
/// each further failure locks the key for an exponentially growing
/// interval (1 s, 2 s, 4 s, … capped at 15 min). A success clears the
/// account key but not the client key, so one valid login can't reset
/// a spraying client. State is per process and lost on restart.
#[derive(Debug, Default)]
pub struct LoginThrottle {
    entries: Mutex<HashMap<Key, Entry>>,
}

impl LoginThrottle {
    /// `Err(retry_after)` when `account` or `client` is locked out; the
    /// caller must refuse the attempt without checking the password.
    pub fn check(&self, account: &str, client: Option<ClientAddr>) -> Result<(), Duration> {
        self.check_at(account, client, Instant::now())
    }

    /// Count a failed attempt against `account` and `client`.
    pub fn record_failure(&self, account: &str, client: Option<ClientAddr>) {
        self.record_failure_at(account, client, Instant::now());
    }

    /// Clear `account`'s failure history after a successful attempt.
    pub fn record_success(&self, account: &str) {
        self.lock().remove(&Key::Account(normalize(account)));
    }

    fn check_at(
        &self,
        account: &str,
        client: Option<ClientAddr>,
        now: Instant,
    ) -> Result<(), Duration> {
        let entries = self.lock();
        let retry_after = keys(account, client)
            .filter_map(|key| entries.get(&key)?.locked_until)
            .filter(|until| *until > now)
            .map(|until| until - now)
            .max();
        match retry_after {
            Some(wait) => Err(wait),
            None => Ok(()),
        }
    }

    fn record_failure_at(&self, account: &str, client: Option<ClientAddr>, now: Instant) {
        let mut entries = self.lock();
        for key in keys(account, client) {
            if !entries.contains_key(&key) && entries.len() >= MAX_ENTRIES {
                make_room(&mut entries, now);
            }
            let entry = entries.entry(key).or_insert(Entry {
                failures: 0,
                last_failure: now,
                locked_until: None,
            });
            if entry.expired(now) {
                entry.failures = 0;
                entry.locked_until = None;
            }
            entry.failures = entry.failures.saturating_add(1);
            entry.last_failure = now;
            if entry.failures > FREE_FAILURES {
                let exponent = (entry.failures - FREE_FAILURES - 1).min(20);
                let lockout = Duration::from_secs(1u64 << exponent).min(MAX_LOCKOUT);
                entry.locked_until = Some(now + lockout);
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Key, Entry>> {
        // The map holds plain counters; a panic mid-update can't leave
        // it in a state worse than "one failure miscounted".
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn normalize(account: &str) -> String {
    account.to_lowercase()
}

fn keys(account: &str, client: Option<ClientAddr>) -> impl Iterator<Item = Key> {
    std::iter::once(Key::Account(normalize(account)))
        .chain(client.map(|ClientAddr(ip)| Key::Client(ip)))
}

/// Drop expired entries; if the map is still full, evict the entry with
/// the oldest failure.
fn make_room(entries: &mut HashMap<Key, Entry>, now: Instant) {
    entries.retain(|_, entry| !entry.expired(now));
    if entries.len() < MAX_ENTRIES {
        return;
    }
    if let Some(oldest) = entries
        .iter()
        .min_by_key(|(_, entry)| entry.last_failure)
        .map(|(key, _)| key.clone())
    {
        entries.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IP: ClientAddr = ClientAddr(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));

    #[test]
    fn locks_after_free_failures_with_exponential_backoff() {
        let throttle = LoginThrottle::default();
        let start = Instant::now();
        for _ in 0..FREE_FAILURES {
            assert!(throttle.check_at("alice", None, start).is_ok());
            throttle.record_failure_at("alice", None, start);
        }
        assert!(throttle.check_at("alice", None, start).is_ok());
        throttle.record_failure_at("alice", None, start);
        assert_eq!(
            throttle.check_at("ALICE", None, start),
            Err(Duration::from_secs(1))
        );
        let later = start + Duration::from_secs(1);
        assert!(throttle.check_at("alice", None, later).is_ok());
        throttle.record_failure_at("alice", None, later);
        assert_eq!(
            throttle.check_at("alice", None, later),
            Err(Duration::from_secs(2))
        );
    }

    #[test]
    fn success_clears_account_but_not_client() {
        let throttle = LoginThrottle::default();
        let now = Instant::now();
        for _ in 0..=FREE_FAILURES {
            throttle.record_failure_at("bob", Some(IP), now);
        }
        assert!(throttle.check_at("bob", None, now).is_err());
        throttle.record_success("bob");
        assert!(throttle.check_at("bob", None, now).is_ok());
        assert!(throttle.check_at("carol", Some(IP), now).is_err());
    }

    #[test]
    fn window_expiry_resets_failures() {
        let throttle = LoginThrottle::default();
        let start = Instant::now();
        for _ in 0..FREE_FAILURES {
            throttle.record_failure_at("dave", None, start);
        }
        let later = start + WINDOW;
        throttle.record_failure_at("dave", None, later);
        assert!(throttle.check_at("dave", None, later).is_ok());
    }

    #[test]
    fn map_stays_bounded() {
        let throttle = LoginThrottle::default();
        let now = Instant::now();
        for i in 0..(MAX_ENTRIES + 10) {
            throttle.record_failure_at(&format!("user{i}"), None, now);
        }
        assert!(throttle.lock().len() <= MAX_ENTRIES);
    }
}
