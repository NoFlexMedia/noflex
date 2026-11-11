// SPDX-License-Identifier: AGPL-3.0-only
//
// Host-scheduled background tasks. A component only executes while the
// host is driving a call, so periodic work is host-scheduled: one tokio
// task per manifest `[[tasks]]` entry, invoking `background.run-task`
// through the plugin's existing instance channel. Background calls ride
// the same one-instance `run_concurrent` loop as foreground hooks, so
// guest state stays single-threaded-coherent and there is no second
// dispatch path.
//
// A task has one `more(cursor)` chain run unbound, for its schedule and
// for unbound enqueues, and one per user whose call enqueued a payload,
// run bound to them. Each iteration runs every chain that is due at once.
// A `per_user` task's schedule starts such a chain for every linked user
// who has none running, instead of the unbound run.
//
// Cursors (with the user each chain is bound to), failure counts, and
// dynamically enqueued payloads persist in storage (`plugin_task_state` /
// `plugin_task_queue`), so they survive instance recycling and host
// restarts. When no storage is configured (tests, minimal embeds) the loop
// keeps state in memory and dynamic enqueue is unavailable — scheduled
// firing still works.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
    time::Duration,
};

use noflex_storage::{PluginTaskState, Storage};
use time::OffsetDateTime;
use tokio::task::JoinHandle;

use crate::{
    LoadedPlugin, audit,
    manifest::TaskRegistration,
    plugin::{PluginHealth, TaskOutcome},
};

/// Marks a queue row written by this host's envelope encoder. Rows without
/// it decode as an unbound raw payload, so a guest-chosen payload string can
/// never claim a principal.
const QUEUED_ENVELOPE_PREFIX: &str = "noflex-task/1:";

/// Host envelope around a dynamically enqueued payload: the guest payload
/// plus the user whose call enqueued it (if any). The storage row stays an
/// opaque string; only the host reads the envelope.
#[derive(serde::Serialize, serde::Deserialize)]
struct QueuedEnvelope {
    user: Option<noflex_core::UserId>,
    payload: Option<String>,
}

/// Encode a queue row. `(None, None)` stays `None`, the historical shape.
pub(crate) fn encode_queued(
    user: Option<noflex_core::UserId>,
    payload: Option<String>,
) -> Option<String> {
    if user.is_none() && payload.is_none() {
        return None;
    }
    // Serializing a struct of strings and UUIDs cannot fail.
    let json = serde_json::to_string(&QueuedEnvelope { user, payload }).ok()?;
    Some(format!("{QUEUED_ENVELOPE_PREFIX}{json}"))
}

/// Decode a queue row into `(enqueuing user, guest payload)`. Rows that are
/// not a well-formed envelope are treated as an unbound raw payload.
pub(crate) fn decode_queued(row: Option<String>) -> (Option<noflex_core::UserId>, Option<String>) {
    let Some(row) = row else {
        return (None, None);
    };
    match row
        .strip_prefix(QUEUED_ENVELOPE_PREFIX)
        .and_then(|json| serde_json::from_str::<QueuedEnvelope>(json).ok())
    {
        Some(envelope) => (envelope.user, envelope.payload),
        None => (None, Some(row)),
    }
}

/// Marks a task-state cursor that holds [`Chains`] rather than the
/// unbound chain's cursor alone.
const CHAINS_ENVELOPE_PREFIX: &str = "noflex-chains/1:";

/// Every `more(cursor)` chain of one task: the unbound one (scheduled runs
/// and unbound enqueues) and one per user whose call enqueued a payload.
/// A user's chain runs bound to them, so it can reach their per-user
/// secrets, and nobody else's run continues or replaces it.
#[derive(Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Chains {
    unbound: Option<String>,
    users: HashMap<noflex_core::UserId, String>,
}

impl Chains {
    /// The persisted cursor column: the unbound cursor as is while no user
    /// has a chain, else an envelope. A guest cursor that looks like an
    /// envelope is enveloped too, so a guest can never claim a principal.
    fn encode(&self) -> Option<String> {
        let plain = self.users.is_empty()
            && self
                .unbound
                .as_deref()
                .is_none_or(|cursor| !cursor.starts_with(CHAINS_ENVELOPE_PREFIX));
        if plain {
            return self.unbound.clone();
        }
        // Serializing strings and UUIDs cannot fail.
        let json = serde_json::to_string(self).ok()?;
        Some(format!("{CHAINS_ENVELOPE_PREFIX}{json}"))
    }

    /// Reverses [`Self::encode`]; anything but a well-formed envelope is
    /// the unbound chain's cursor.
    fn decode(stored: Option<String>) -> Self {
        let Some(stored) = stored else {
            return Self::default();
        };
        stored
            .strip_prefix(CHAINS_ENVELOPE_PREFIX)
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or(Self {
                unbound: Some(stored),
                users: HashMap::new(),
            })
    }

    fn cursor(&self, user: Option<noflex_core::UserId>) -> Option<String> {
        match user {
            None => self.unbound.clone(),
            Some(user) => self.users.get(&user).cloned(),
        }
    }

    fn set(&mut self, user: Option<noflex_core::UserId>, cursor: Option<String>) {
        match (user, cursor) {
            (None, cursor) => self.unbound = cursor,
            (Some(user), Some(cursor)) => {
                self.users.insert(user, cursor);
            }
            (Some(user), None) => {
                self.users.remove(&user);
            }
        }
    }
}

/// One `run-task` call an iteration makes: the chain it continues (or
/// starts) and what it hands the guest.
struct Run {
    user: Option<noflex_core::UserId>,
    cursor: Option<String>,
    payload: Option<String>,
}

/// Upper bound on how long a queued dynamic payload waits before the loop
/// notices it. Interval-driven firing is unaffected; this only bounds the
/// poll latency of the dynamic-enqueue queue, which has no wakeup channel.
const QUEUE_POLL: Duration = Duration::from_millis(500);

/// Spawn one scheduler task per manifest `[[tasks]]` entry. The returned
/// handles are owned by the registry, which aborts them on unload or
/// hot-reload replace; each loop also exits on its own once the plugin's
/// `Arc` is gone or its instance channel closes.
pub(crate) fn spawn_for_plugin(
    plugin: &Arc<LoadedPlugin>,
    storage: Option<Arc<dyn Storage>>,
) -> Vec<JoinHandle<()>> {
    plugin
        .manifest
        .tasks
        .iter()
        .map(|reg| {
            let reg = reg.clone();
            let weak = Arc::downgrade(plugin);
            let plugin_id = plugin.id.0.clone();
            let health = Arc::clone(&plugin.health);
            let storage = storage.clone();
            tokio::spawn(run_task_loop(weak, plugin_id, reg, health, storage))
        })
        .collect()
}

async fn run_task_loop(
    weak: Weak<LoadedPlugin>,
    plugin_id: String,
    reg: TaskRegistration,
    health: Arc<std::sync::RwLock<PluginHealth>>,
    storage: Option<Arc<dyn Storage>>,
) {
    audit::task_run(
        &plugin_id,
        &reg.name,
        "start",
        &format!("interval_ms={}", reg.interval_ms),
    );

    // Restore the persisted chains so a long sweep resumes where the last
    // instance (or host process) left off, still bound to its user.
    let mut stored = None;
    if let Some(storage) = &storage {
        match storage.plugin_task_state(&plugin_id, &reg.name).await {
            Ok(Some(state)) => stored = state.cursor,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(plugin = %plugin_id, task = %reg.name, error = %e,
                    "task state restore failed; starting with no cursor");
            }
        }
    }
    let mut chains = Chains::decode(stored);
    let mut consecutive_failures: u32 = 0;

    // Delay on missed ticks: a slow invocation shifts the schedule instead
    // of bursting catch-up calls.
    let mut interval = tokio::time::interval(Duration::from_millis(reg.interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first interval tick is immediate; consume it so the first
    // scheduled fire happens one full interval after load.
    interval.tick().await;

    // Chains whose guest answered `more(cursor)`: re-invoked promptly,
    // skipping the interval wait, so chunked sweeps keep moving. A user's
    // restored chain resumes at once, as whatever it waits on is theirs.
    let mut prompt_unbound = false;
    let mut prompt_users: HashSet<noflex_core::UserId> = chains.users.keys().copied().collect();
    // Users whose chains were cancelled while they unlink, whose secrets may
    // not be gone yet: a per-user tick mustn't start them a new chain. They
    // leave once they're seen unlinked, or an enqueue of theirs starts one.
    let mut unlinking: HashSet<noflex_core::UserId> = HashSet::new();

    let end_reason = loop {
        let mut tick = false;
        if !prompt_unbound && prompt_users.is_empty() {
            tokio::select! {
                _ = interval.tick() => tick = true,
                _ = tokio::time::sleep(QUEUE_POLL) => {}
            }
        }

        let Some(plugin) = weak.upgrade() else {
            break "plugin unloaded";
        };
        if plugin.instance_closed() {
            break "instance stopped";
        }

        let cancelled = cancelled_users(&plugin, &reg.name, &mut chains, &mut prompt_users);
        let changed = !cancelled.is_empty();
        unlinking.extend(cancelled);

        // Dynamic payloads (host-tasks.enqueue) take priority over the
        // scheduled payload on their chain.
        let dequeued = match &storage {
            Some(storage) => match storage.dequeue_plugin_task(&plugin_id, &reg.name).await {
                Ok(p) => p.map(|p| decode_queued(p.payload)),
                Err(e) => {
                    tracing::warn!(plugin = %plugin_id, task = %reg.name, error = %e,
                        "task queue dequeue failed");
                    None
                }
            },
            None => None,
        };

        let mut runs: Vec<Run> = Vec::new();
        if let Some((user, payload)) = dequeued {
            if let Some(user) = user {
                unlinking.remove(&user);
            }
            runs.push(Run {
                user,
                cursor: chains.cursor(user),
                payload,
            });
        }
        let covered = |runs: &[Run], user| runs.iter().any(|r| r.user == user);
        // A per-user task's schedule starts a chain for each linked user
        // instead of running unbound; users mid-chain carry on below.
        let unbound_tick = tick && !reg.per_user;
        if tick && reg.per_user {
            let linked = linked_users(&storage, &plugin_id).await;
            unlinking.retain(|user| linked.contains(user));
            for user in linked {
                if !unlinking.contains(&user)
                    && !chains.users.contains_key(&user)
                    && !covered(&runs, Some(user))
                {
                    runs.push(Run {
                        user: Some(user),
                        cursor: None,
                        payload: Some(user.to_string()),
                    });
                }
            }
        }
        if (unbound_tick || prompt_unbound) && !covered(&runs, None) {
            runs.push(Run {
                user: None,
                cursor: chains.unbound.clone(),
                payload: reg.payload.clone(),
            });
        }
        // A tick also retries user chains an error left waiting.
        let users: Vec<_> = if tick {
            chains.users.keys().copied().collect()
        } else {
            prompt_users.iter().copied().collect()
        };
        for user in users {
            if !covered(&runs, Some(user)) {
                runs.push(Run {
                    user: Some(user),
                    cursor: chains.cursor(Some(user)),
                    payload: reg.payload.clone(),
                });
            }
        }
        prompt_unbound = false;
        prompt_users.clear();

        if runs.is_empty() {
            if changed {
                persist(
                    &storage,
                    &plugin_id,
                    &reg.name,
                    &chains,
                    None,
                    consecutive_failures,
                )
                .await;
            }
            continue;
        }

        let outcomes = futures::future::join_all(runs.iter().map(|run| {
            plugin.call_run_task_as(
                run.user,
                reg.name.clone(),
                run.cursor.clone(),
                run.payload.clone(),
            )
        }))
        .await;
        let now = OffsetDateTime::now_utc();
        // Cancelled mid-run: their run's outcome no longer has a chain to land in.
        let cancelled = cancelled_users(&plugin, &reg.name, &mut chains, &mut prompt_users);
        unlinking.extend(cancelled.iter().copied());

        let mut failed = false;
        for (run, outcome) in runs.iter().zip(outcomes) {
            if run.user.is_some_and(|user| cancelled.contains(&user)) {
                continue;
            }
            match outcome {
                Ok(Ok(TaskOutcome::Done)) => chains.set(run.user, None),
                Ok(Ok(TaskOutcome::More(next))) => {
                    chains.set(run.user, Some(next));
                    match run.user {
                        None => prompt_unbound = true,
                        Some(user) => {
                            prompt_users.insert(user);
                        }
                    }
                }
                // A plugin-error keeps the chain's cursor: it retries from
                // the same position on the next tick.
                Ok(Err(e)) => {
                    failed = true;
                    tracing::warn!(plugin = %plugin_id, task = %reg.name, error = ?e,
                        "background task returned an error");
                }
                Err(e) => {
                    failed = true;
                    tracing::warn!(plugin = %plugin_id, task = %reg.name, error = %e,
                        "background task dispatch failed");
                }
            }
        }
        consecutive_failures = if failed { consecutive_failures + 1 } else { 0 };

        {
            let mut h = health.write().expect("health lock poisoned");
            if let Some(t) = h.tasks.get_mut(&reg.name) {
                t.last_run_at = Some(now);
                t.consecutive_failures = consecutive_failures;
                t.total_runs += runs.len() as u64;
            }
        }
        persist(
            &storage,
            &plugin_id,
            &reg.name,
            &chains,
            Some(now),
            consecutive_failures,
        )
        .await;

        if plugin.instance_closed() {
            break "instance stopped";
        }
    };

    audit::task_run(&plugin_id, &reg.name, "end", end_reason);
}

/// Everyone who has linked an account through `plugin_id`; none without
/// storage, or when reading it fails (the next tick asks again).
async fn linked_users(
    storage: &Option<Arc<dyn Storage>>,
    plugin_id: &str,
) -> Vec<noflex_core::UserId> {
    let Some(storage) = storage else {
        return Vec::new();
    };
    match storage.plugin_linked_users(plugin_id).await {
        Ok(users) => users,
        Err(e) => {
            tracing::warn!(plugin = %plugin_id, error = %e, "per-user task: linked users unreadable");
            Vec::new()
        }
    }
}

/// Apply the cancellations waiting for `task`: drop each user's chain and
/// answer the canceller. Returns the users whose chains were cancelled.
fn cancelled_users(
    plugin: &LoadedPlugin,
    task: &str,
    chains: &mut Chains,
    prompt_users: &mut HashSet<noflex_core::UserId>,
) -> HashSet<noflex_core::UserId> {
    let mut users = HashSet::new();
    for (user, ack) in plugin.take_task_cancels(task) {
        chains.users.remove(&user);
        prompt_users.remove(&user);
        users.insert(user);
        // The canceller may have stopped waiting.
        let _ = ack.send(());
    }
    users
}

async fn persist(
    storage: &Option<Arc<dyn Storage>>,
    plugin_id: &str,
    task: &str,
    chains: &Chains,
    last_run_at: Option<OffsetDateTime>,
    consecutive_failures: u32,
) {
    let Some(storage) = storage else {
        return;
    };
    let last_run_at = match last_run_at {
        Some(at) => Some(at),
        None => match storage.plugin_task_state(plugin_id, task).await {
            Ok(state) => state.and_then(|s| s.last_run_at),
            Err(_) => None,
        },
    };
    let state = PluginTaskState {
        plugin_id: plugin_id.to_string(),
        task: task.to_string(),
        cursor: chains.encode(),
        last_run_at,
        consecutive_failures,
    };
    if let Err(e) = storage.set_plugin_task_state(&state).await {
        tracing::warn!(plugin = %plugin_id, task = %task, error = %e,
            "task state persist failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_envelope_round_trips_user_and_payload() {
        let user = noflex_core::UserId::from_uuid(uuid::Uuid::now_v7());
        let row = encode_queued(Some(user), Some("{\"x\":1}".into()));
        assert_eq!(decode_queued(row), (Some(user), Some("{\"x\":1}".into())));
        assert_eq!(encode_queued(None, None), None);
        assert_eq!(decode_queued(None), (None, None));
    }

    #[test]
    fn chains_round_trip_with_their_users() {
        let user = noflex_core::UserId::from_uuid(uuid::Uuid::now_v7());
        let mut chains = Chains::default();
        assert_eq!(chains.encode(), None);
        chains.set(None, Some("7".into()));
        assert_eq!(
            chains.encode().as_deref(),
            Some("7"),
            "unbound alone stays plain"
        );
        chains.set(Some(user), Some("{\"poll\":1}".into()));
        assert_eq!(Chains::decode(chains.encode()), chains);
        chains.set(Some(user), None);
        assert_eq!(chains.encode().as_deref(), Some("7"));
    }

    #[test]
    fn a_guest_cursor_never_binds_a_chain() {
        let victim = noflex_core::UserId::from_uuid(uuid::Uuid::now_v7());
        let forged = format!(
            "{CHAINS_ENVELOPE_PREFIX}{{\"unbound\":null,\"users\":{{\"{victim}\":\"x\"}}}}"
        );
        let mut chains = Chains::default();
        chains.set(None, Some(forged.clone()));
        let restored = Chains::decode(chains.encode());
        assert!(restored.users.is_empty());
        assert_eq!(restored.unbound, Some(forged));
    }

    #[test]
    fn raw_rows_never_carry_a_principal() {
        let user = noflex_core::UserId::from_uuid(uuid::Uuid::now_v7());
        let forged = format!("{{\"user\":\"{user}\",\"payload\":null}}");
        assert_eq!(decode_queued(Some(forged.clone())), (None, Some(forged)));
        let truncated = format!("{QUEUED_ENVELOPE_PREFIX}{{");
        assert_eq!(
            decode_queued(Some(truncated.clone())),
            (None, Some(truncated))
        );
    }
}
