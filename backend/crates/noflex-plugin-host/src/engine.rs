// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use wasmtime::{Config, Engine};

/// Tunables for the shared Wasmtime engine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Default per-call wall-clock deadline (seconds), measured from when
    /// the instance starts the call. Plugins override per hook via the
    /// manifest's `[deadlines_secs]`. A call still executing past it is
    /// interrupted at the next epoch tick; a call parked past it is cut
    /// off by a timer. Either way the instance is recycled.
    pub default_epoch_deadline_secs: u32,
    /// Default per-plugin memory cap in MiB. Plugins can override.
    pub default_memory_limit_mb: u32,
    /// Engine epoch ticker period (ms): how often running guest code
    /// yields to the executor and checks its call's deadline.
    pub epoch_ticker_period_ms: u64,
    /// Enable `wasmtime` debug info embedding so traps include source maps
    /// when plugins were built with `debug = "line-tables-only"` or better.
    pub debug_info: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            default_epoch_deadline_secs: 30,
            default_memory_limit_mb: 64,
            epoch_ticker_period_ms: 100,
            debug_info: false,
        }
    }
}

/// Build a [`wasmtime::Engine`] configured for NoFlex plugin execution.
///
/// Component model + native async (WASI 0.3) + epoch interruption are
/// mandatory. Concurrency support is what allows one plugin instance to
/// service many overlapping hook calls inside a single
/// [`wasmtime::Store::run_concurrent`] scope. The engine is shared across
/// all plugins; per-plugin state lives in [`wasmtime::Store`].
pub fn build_engine(cfg: &EngineConfig) -> Result<Arc<Engine>, wasmtime::Error> {
    let mut config = Config::new();
    config
        .epoch_interruption(true)
        .wasm_component_model(true)
        .wasm_component_model_async(true)
        .concurrency_support(true)
        .debug_info(cfg.debug_info)
        .cranelift_opt_level(wasmtime::OptLevel::Speed);

    let engine = Engine::new(&config)?;
    Ok(Arc::new(engine))
}

/// Spawn a background tokio task that ticks [`Engine::increment_epoch`] every
/// `cfg.epoch_ticker_period_ms`. Deadline enforcement for guest code that
/// never yields depends on these ticks, so run the server on a
/// multi-threaded runtime: a spinning guest occupies its worker thread.
///
/// Returns a [`tokio::task::JoinHandle`] so the caller can shut down the
/// ticker on graceful exit; dropping the handle does NOT cancel the task
/// (it lives for the engine's lifetime by default).
pub fn spawn_epoch_ticker(engine: Arc<Engine>, period_ms: u64) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_millis(period_ms.max(1)));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            engine.increment_epoch();
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_builds_with_default_config() {
        let cfg = EngineConfig::default();
        let engine = build_engine(&cfg).expect("engine builds");
        // Sanity: engine reports the configured features. Just confirm it
        // hasn't panicked or returned an obviously-broken config.
        assert!(Arc::strong_count(&engine) >= 1);
    }
}
