// SPDX-License-Identifier: AGPL-3.0-only
//
// Per-plugin resource limits enforced via wasmtime's [`ResourceLimiter`].
// Memory growth is the only hard cap we enforce today; table growth is
// bounded but not tightly limited (plugins rarely hit table caps in practice).

use wasmtime::ResourceLimiter;

/// Memory + table caps applied per plugin instance.
#[derive(Debug, Clone)]
pub struct PluginLimits {
    pub max_memory_bytes: usize,
    pub max_tables: usize,
    pub max_table_elements: usize,
    pub max_instances: usize,
    pub max_memories: usize,
}

impl PluginLimits {
    /// Construct limits from a memory budget in mebibytes.
    pub fn from_mb(mb: u32) -> Self {
        let bytes = (mb as usize).saturating_mul(1024 * 1024);
        Self {
            max_memory_bytes: bytes,
            // These generous defaults leave headroom for plugins that use
            // multiple memories or tables.
            max_tables: 4,
            max_table_elements: 100_000,
            max_instances: 8,
            max_memories: 4,
        }
    }
}

impl Default for PluginLimits {
    fn default() -> Self {
        Self::from_mb(64)
    }
}

/// State threaded into the wasmtime `Store::limiter` callback. Tracks
/// current allocation so trace events can include "tried to grow to N MB"
/// context when a plugin trips the cap.
pub struct PluginLimitState {
    limits: PluginLimits,
    plugin_id: String,
    allocated_memory_bytes: usize,
    pending_growth_bytes: Option<usize>,
}

impl PluginLimitState {
    pub fn new(plugin_id: impl Into<String>, limits: PluginLimits) -> Self {
        Self {
            limits,
            plugin_id: plugin_id.into(),
            allocated_memory_bytes: 0,
            pending_growth_bytes: None,
        }
    }
}

impl ResourceLimiter for PluginLimitState {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.pending_growth_bytes = None;
        let growth = desired.saturating_sub(current);
        let total = self.allocated_memory_bytes.saturating_add(growth);
        if total > self.limits.max_memory_bytes {
            tracing::warn!(
                target: "noflex_plugin::audit",
                plugin = %self.plugin_id,
                event = "memory_grow_denied",
                desired_bytes = desired,
                total_bytes = total,
                cap_bytes = self.limits.max_memory_bytes,
            );
            Ok(false)
        } else {
            self.allocated_memory_bytes = total;
            self.pending_growth_bytes = Some(growth);
            Ok(true)
        }
    }

    fn memory_grow_failed(&mut self, _error: wasmtime::Error) -> wasmtime::Result<()> {
        if let Some(growth) = self.pending_growth_bytes.take() {
            self.allocated_memory_bytes = self.allocated_memory_bytes.saturating_sub(growth);
        }
        Ok(())
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.limits.max_table_elements {
            tracing::warn!(
                target: "noflex_plugin::audit",
                plugin = %self.plugin_id,
                event = "table_grow_denied",
                desired_elements = desired,
                cap_elements = self.limits.max_table_elements,
            );
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn instances(&self) -> usize {
        self.limits.max_instances
    }

    fn tables(&self) -> usize {
        self.limits.max_tables
    }

    fn memories(&self) -> usize {
        self.limits.max_memories
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_from_mb_translates_to_bytes() {
        let l = PluginLimits::from_mb(32);
        assert_eq!(l.max_memory_bytes, 32 * 1024 * 1024);
    }

    #[test]
    fn limiter_blocks_growth_past_cap() {
        let limits = PluginLimits::from_mb(1);
        let mut state = PluginLimitState::new("test", limits.clone());
        assert!(state.memory_growing(0, 512 * 1024, None).unwrap());
        assert!(!state.memory_growing(0, 2 * 1024 * 1024, None).unwrap());
    }

    #[test]
    fn wasmtime_store_enforces_memory_cap() {
        let engine = wasmtime::Engine::default();
        let mut store = wasmtime::Store::new(
            &engine,
            PluginLimitState::new("test", PluginLimits::from_mb(1)),
        );
        store.limiter(|state| state);
        let memory = wasmtime::MemoryType::new(32, None);
        assert!(wasmtime::Memory::new(&mut store, memory).is_err());
    }
}
