// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{
    error::PluginResult,
    ids::{UserId, WorkId},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotifyEvent {
    WorkAdded {
        work: WorkId,
        title: String,
    },
    WorkPlayed {
        user: UserId,
        work: WorkId,
        title: String,
    },
    ScanStarted {
        library_name: String,
    },
    ScanCompleted {
        library_name: String,
        added: usize,
        updated: usize,
        removed: usize,
    },
    Custom {
        event: String,
        payload: serde_json::Value,
    },
}

#[async_trait]
pub trait Notifier: Send + Sync {
    fn id(&self) -> &str;

    async fn notify(&self, event: &NotifyEvent) -> PluginResult<()>;
}
