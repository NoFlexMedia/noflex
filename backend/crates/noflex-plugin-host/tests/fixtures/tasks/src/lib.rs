// SPDX-License-Identifier: AGPL-3.0-only
//
// Test-fixture plugin for noflex-plugin-host's background-task suite. The
// "sweep" task never finishes: every invocation bumps the numeric cursor
// and answers `more`, so the host-side cursor advances promptly without
// waiting on the manifest interval. `payload == "boom"` traps the guest
// so tests can force an instance recycle; `payload == "dynamic"` proves a
// `host-tasks.enqueue` payload reached the task. "self-enqueue" exercises
// the enqueue import from inside a `run-task` call.
//
// The counters are instance-global statics on purpose: a recycle
// re-instantiates the component and zeroes them, which is exactly what
// lets the host tests prove the *host-side* cursor survives recycling.
// `metadata.search` re-exports the counters in the match title so the
// host has a guest-state observation channel without a dedicated export.

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "metadata-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use std::sync::atomic::{AtomicU32, Ordering};

use exports::noflex::plugin::{background, metadata};
use noflex::plugin::{host_tasks, types::ExternalIdKind};

static SWEEP_RUNS: AtomicU32 = AtomicU32::new(0);
static DYNAMIC_RUNS: AtomicU32 = AtomicU32::new(0);

struct Fixture;

impl background::Guest for Fixture {
    async fn run_task(
        task: String,
        cursor: Option<String>,
        payload: Option<String>,
    ) -> Result<background::TaskOutcome, background::PluginError> {
        match task.as_str() {
            "sweep" => {
                if payload.as_deref() == Some("boom") {
                    panic!("fixture-tasks: boom");
                }
                SWEEP_RUNS.fetch_add(1, Ordering::Relaxed);
                if payload.as_deref() == Some("dynamic") {
                    DYNAMIC_RUNS.fetch_add(1, Ordering::Relaxed);
                }
                let n = cursor
                    .as_deref()
                    .and_then(|c| c.parse::<u32>().ok())
                    .unwrap_or(0)
                    + 1;
                Ok(background::TaskOutcome::More(n.to_string()))
            }
            "self-enqueue" => {
                match host_tasks::enqueue("sweep".to_string(), Some("dynamic".to_string())).await {
                    Ok(()) => Ok(background::TaskOutcome::Done),
                    Err(e) => Err(background::PluginError::BadInput(format!(
                        "fixture-tasks: enqueue failed: {e:?}"
                    ))),
                }
            }
            "self-enqueue-unknown" => {
                match host_tasks::enqueue("unknown".to_string(), None).await {
                    Ok(()) => Err(background::PluginError::Internal(
                        "unknown enqueue unexpectedly succeeded".into(),
                    )),
                    Err(e) => Err(e),
                }
            }
            "self-enqueue-oversize" => {
                match host_tasks::enqueue("sweep".to_string(), Some("x".repeat(1024 * 1024 + 1)))
                    .await
                {
                    Ok(()) => Err(background::PluginError::Internal(
                        "oversize enqueue unexpectedly succeeded".into(),
                    )),
                    Err(e) => Err(e),
                }
            }
            other => Err(background::PluginError::BadInput(format!(
                "fixture-tasks: unknown task `{other}`"
            ))),
        }
    }
}

impl metadata::Guest for Fixture {
    fn supports(kind: metadata::WorkKind) -> bool {
        matches!(kind, metadata::WorkKind::Movie)
    }

    async fn search(
        _query: metadata::SearchQuery,
    ) -> Result<Vec<metadata::MetadataMatch>, metadata::PluginError> {
        Ok(vec![metadata::MetadataMatch {
            external_id: metadata::ExternalId {
                kind: ExternalIdKind::Custom,
                value: "fixture-tasks:probe".into(),
            },
            title: format!(
                "runs={} dynamic={}",
                SWEEP_RUNS.load(Ordering::Relaxed),
                DYNAMIC_RUNS.load(Ordering::Relaxed)
            ),
            year: None,
            confidence: 1.0,
        }])
    }

    async fn fetch(
        _id: metadata::ExternalId,
    ) -> Result<metadata::MetadataPayload, metadata::PluginError> {
        Err(metadata::PluginError::BadInput(
            "fixture-tasks: fetch not implemented".into(),
        ))
    }

    async fn fetch_children(
        _parent: metadata::ExternalId,
        _parent_work: metadata::WorkId,
    ) -> Result<Vec<metadata::MetadataPayload>, metadata::PluginError> {
        Ok(Vec::new())
    }
}

export!(Fixture);
