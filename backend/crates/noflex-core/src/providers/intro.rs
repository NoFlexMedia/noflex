// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{error::PluginResult, ids::EditionId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntroDetection {
    pub intro_start_ms: Option<i64>,
    pub intro_end_ms: Option<i64>,
    pub credits_start_ms: Option<i64>,
    /// Detector-specific confidence in [0.0, 1.0]; informational — the host
    /// persists any returned detection regardless.
    #[serde(default)]
    pub confidence: f32,
}

/// Intro / credits marker detection. Host-driven: a host worker sweeps
/// editions that have never been through detection, calls `detect`, and
/// persists the outcome itself — plugins never see storage. Detection
/// inputs (chapters, runtime, existing markers) arrive through narrow host
/// imports, not a media path.
#[async_trait]
pub trait IntroDetector: Send + Sync {
    fn id(&self) -> &str;

    /// Returns `Ok(None)` when the detector finds nothing worth marking.
    async fn detect(&self, edition: EditionId) -> PluginResult<Option<IntroDetection>>;
}
