// SPDX-License-Identifier: AGPL-3.0-only

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use noflex_core::LibraryId;
use time::OffsetDateTime;
use tokio::sync::RwLock;

use crate::{ScanSummary, Scanner};

#[derive(Debug, Clone)]
pub enum ScanStatus {
    Idle,
    Running {
        started_at: OffsetDateTime,
    },
    Completed {
        started_at: OffsetDateTime,
        finished_at: OffsetDateTime,
        summary: ScanSummary,
    },
    Failed {
        started_at: OffsetDateTime,
        finished_at: OffsetDateTime,
        error: String,
    },
}

impl ScanStatus {
    pub fn state(&self) -> &'static str {
        match self {
            ScanStatus::Idle => "IDLE",
            ScanStatus::Running { .. } => "RUNNING",
            ScanStatus::Completed { .. } => "COMPLETED",
            ScanStatus::Failed { .. } => "FAILED",
        }
    }

    pub fn started_at(&self) -> Option<OffsetDateTime> {
        match self {
            ScanStatus::Idle => None,
            ScanStatus::Running { started_at }
            | ScanStatus::Completed { started_at, .. }
            | ScanStatus::Failed { started_at, .. } => Some(*started_at),
        }
    }

    pub fn finished_at(&self) -> Option<OffsetDateTime> {
        match self {
            ScanStatus::Idle | ScanStatus::Running { .. } => None,
            ScanStatus::Completed { finished_at, .. } | ScanStatus::Failed { finished_at, .. } => {
                Some(*finished_at)
            }
        }
    }

    pub fn summary(&self) -> Option<&ScanSummary> {
        match self {
            ScanStatus::Completed { summary, .. } => Some(summary),
            _ => None,
        }
    }
}

#[async_trait]
pub trait ScanStatusReader: Send + Sync {
    async fn status(&self, library: LibraryId) -> ScanStatus;
}

struct Inner {
    scanner: Arc<dyn Scanner>,
    jobs: RwLock<HashMap<LibraryId, ScanStatus>>,
}

pub struct ScanRegistry {
    inner: Arc<Inner>,
}

impl ScanRegistry {
    pub fn new(scanner: Arc<dyn Scanner>) -> Self {
        Self {
            inner: Arc::new(Inner {
                scanner,
                jobs: RwLock::new(HashMap::new()),
            }),
        }
    }

    pub async fn status(&self, library: LibraryId) -> ScanStatus {
        let guard = self.inner.jobs.read().await;
        guard.get(&library).cloned().unwrap_or(ScanStatus::Idle)
    }

    /// Kick off a scan. Returns the in-flight status if a scan is
    /// already running, otherwise returns the freshly-recorded
    /// `Running` status.
    pub async fn trigger(&self, library: LibraryId) -> ScanStatus {
        let started = OffsetDateTime::now_utc();
        {
            let mut guard = self.inner.jobs.write().await;
            if let Some(running @ ScanStatus::Running { .. }) = guard.get(&library) {
                return running.clone();
            }
            guard.insert(
                library,
                ScanStatus::Running {
                    started_at: started,
                },
            );
        }
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let result = inner.scanner.scan_library(library).await;
            let finished = OffsetDateTime::now_utc();
            let mut guard = inner.jobs.write().await;
            match result {
                Ok(summary) => {
                    guard.insert(
                        library,
                        ScanStatus::Completed {
                            started_at: started,
                            finished_at: finished,
                            summary,
                        },
                    );
                }
                Err(err) => {
                    guard.insert(
                        library,
                        ScanStatus::Failed {
                            started_at: started,
                            finished_at: finished,
                            error: err.to_string(),
                        },
                    );
                }
            }
        });
        ScanStatus::Running {
            started_at: started,
        }
    }
}

#[async_trait]
impl ScanStatusReader for ScanRegistry {
    async fn status(&self, library: LibraryId) -> ScanStatus {
        ScanRegistry::status(self, library).await
    }
}
