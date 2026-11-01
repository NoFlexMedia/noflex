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
// Cursors, failure counts, and dynamically enqueued payloads persist in
// storage (`plugin_task_state` / `plugin_task_queue`), so they survive
// instance recycling and host restarts. When no storage is configured
// (tests, minimal embeds) the loop keeps state in memory and dynamic
// enqueue is unavailable — scheduled firing still works.

use std::{
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

    // Restore the persisted cursor so a long sweep resumes where the last
    // instance (or host process) left off.
    let mut cursor: Option<String> = None;
    if let Some(storage) = &storage {
        match storage.plugin_task_state(&plugin_id, &reg.name).await {
            Ok(Some(state)) => cursor = state.cursor,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(plugin = %plugin_id, task = %reg.name, error = %e,
                    "task state restore failed; starting with no cursor");
            }
        }
    }
    let mut consecutive_failures: u32 = 0;
    // Principal of the `more(cursor)` chain in flight: the user whose call
    // enqueued the payload that started it. In memory only — a chain that
    // resumes from a persisted cursor after a restart runs unbound.
    let mut chain_user: Option<noflex_core::UserId> = None;

    // Delay on missed ticks: a slow invocation shifts the schedule instead
    // of bursting catch-up calls.
    let mut interval = tokio::time::interval(Duration::from_millis(reg.interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first interval tick is immediate; consume it so the first
    // scheduled fire happens one full interval after load.
    interval.tick().await;

    // Set when the guest answered `more(cursor)`: re-invoke promptly,
    // skipping the interval wait, so chunked sweeps keep moving.
    let mut prompt = false;

    let end_reason = loop {
        let mut due = false;
        if prompt {
            due = true;
            prompt = false;
        } else {
            tokio::select! {
                _ = interval.tick() => due = true,
                _ = tokio::time::sleep(QUEUE_POLL) => {}
            }
        }

        let Some(plugin) = weak.upgrade() else {
            break "plugin unloaded";
        };
        if plugin.instance_closed() {
            break "instance stopped";
        }

        // Dynamic payloads (host-tasks.enqueue) take priority over the
        // scheduled payload; a dequeue hit makes this iteration due even
        // when the interval didn't fire.
        let dequeued = match &storage {
            Some(storage) => match storage.dequeue_plugin_task(&plugin_id, &reg.name).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(plugin = %plugin_id, task = %reg.name, error = %e,
                        "task queue dequeue failed");
                    None
                }
            },
            None => None,
        };
        due = due || dequeued.is_some();
        if !due {
            continue;
        }
        let (user, payload) = match dequeued {
            Some(p) => decode_queued(p.payload),
            None => (chain_user, reg.payload.clone()),
        };

        let outcome = plugin
            .call_run_task_as(user, reg.name.clone(), cursor.clone(), payload)
            .await;
        let now = OffsetDateTime::now_utc();
        match outcome {
            Ok(Ok(TaskOutcome::Done)) => {
                cursor = None;
                chain_user = None;
                consecutive_failures = 0;
            }
            Ok(Ok(TaskOutcome::More(next))) => {
                cursor = Some(next);
                chain_user = user;
                consecutive_failures = 0;
                prompt = true;
            }
            // A plugin-error keeps the cursor: the sweep retries from the
            // same position on the next fire.
            Ok(Err(e)) => {
                consecutive_failures += 1;
                tracing::warn!(plugin = %plugin_id, task = %reg.name, error = ?e,
                    "background task returned an error");
            }
            Err(e) => {
                consecutive_failures += 1;
                tracing::warn!(plugin = %plugin_id, task = %reg.name, error = %e,
                    "background task dispatch failed");
            }
        }

        {
            let mut h = health.write().expect("health lock poisoned");
            if let Some(t) = h.tasks.get_mut(&reg.name) {
                t.last_run_at = Some(now);
                t.consecutive_failures = consecutive_failures;
                t.total_runs += 1;
            }
        }

        if let Some(storage) = &storage {
            let state = PluginTaskState {
                plugin_id: plugin_id.clone(),
                task: reg.name.clone(),
                cursor: cursor.clone(),
                last_run_at: Some(now),
                consecutive_failures,
            };
            if let Err(e) = storage.set_plugin_task_state(&state).await {
                tracing::warn!(plugin = %plugin_id, task = %reg.name, error = %e,
                    "task state persist failed");
            }
        }

        if plugin.instance_closed() {
            break "instance stopped";
        }
    };

    audit::task_run(&plugin_id, &reg.name, "end", end_reason);
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
