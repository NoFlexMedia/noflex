// SPDX-License-Identifier: AGPL-3.0-only
//
// Event broadcaster for export-job status updates.
// Fire-and-forget `tokio::sync::broadcast`; slow consumers don't
// block the worker. The GQL `Subscription.exportJobStatus`
// resolver subscribes to this; the server's
// `DownloadWorker` emits.

use noflex_core::ExportJobId;
use noflex_storage::domain::ExportJobStatus;
use noflex_transcoder::reasons::TranscodeReasons;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum ExportEvent {
    StatusChanged {
        id: ExportJobId,
        status: ExportJobStatus,
        progress: f32,
        reasons: Option<TranscodeReasons>,
    },
}

#[derive(Debug, Clone)]
pub struct ExportEventBroadcaster {
    tx: broadcast::Sender<ExportEvent>,
}

impl ExportEventBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ExportEvent> {
        self.tx.subscribe()
    }

    pub fn emit_status(
        &self,
        id: ExportJobId,
        status: ExportJobStatus,
        progress: f32,
        reasons: Option<TranscodeReasons>,
    ) {
        let _ = self.tx.send(ExportEvent::StatusChanged {
            id,
            status,
            progress,
            reasons,
        });
    }
}

impl Default for ExportEventBroadcaster {
    fn default() -> Self {
        Self::new(64)
    }
}
