// SPDX-License-Identifier: AGPL-3.0-only
//
// Runs each library's `scan_schedule`. The last completed scan comes from
// the status recorder, which restores it from the activity log at boot,
// so a restart doesn't reset the clock and trigger a rescan of
// everything.

use std::{sync::Arc, time::Duration};

use noflex_scanner::ScanRegistry;
use noflex_storage::{Storage, domain::ScanSchedule};
use time::OffsetDateTime;

use crate::system_status::Recorder;

/// How often schedules are checked; also the worst-case lateness.
const TICK: Duration = Duration::from_secs(5 * 60);
/// Delay before the first check, so boot-time scans and the recorder's
/// activity-log restore land first.
const FIRST_CHECK: Duration = Duration::from_secs(60);

/// Whether a library on `schedule` whose last scan finished at `last`
/// is due at `now`. A library never scanned is due immediately.
pub fn scan_due(schedule: ScanSchedule, last: Option<OffsetDateTime>, now: OffsetDateTime) -> bool {
    let every = match schedule {
        ScanSchedule::Disabled => return false,
        ScanSchedule::Hourly => time::Duration::HOUR,
        ScanSchedule::Daily => time::Duration::DAY,
        ScanSchedule::Weekly => time::Duration::WEEK,
    };
    last.is_none_or(|last| now - last >= every)
}

/// Spawn the scheduler loop. It never exits on its own; errors are
/// logged and retried on the next tick.
pub fn spawn(
    storage: Arc<dyn Storage>,
    scans: Arc<ScanRegistry>,
    recorder: Arc<Recorder>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            let libraries = match storage.list_libraries().await {
                Ok(libraries) => libraries,
                Err(error) => {
                    tracing::warn!(%error, "scan scheduler: listing libraries failed");
                    continue;
                }
            };
            let now = OffsetDateTime::now_utc();
            for library in libraries {
                let (active, last, _) = recorder.library(library.id);
                if active.is_some() {
                    continue;
                }
                let last = last.and_then(|run| run.finished_at.or(run.started_at));
                if scan_due(library.config.scan_schedule, last, now) {
                    tracing::info!(library = %library.id, schedule = ?library.config.scan_schedule, "scheduled scan");
                    scans.trigger(library.id).await;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_follows_the_schedule_interval() {
        let now = OffsetDateTime::now_utc();
        let ago = |d: time::Duration| Some(now - d);
        assert!(!scan_due(ScanSchedule::Disabled, None, now));
        assert!(scan_due(ScanSchedule::Weekly, None, now), "never scanned");
        assert!(!scan_due(
            ScanSchedule::Hourly,
            ago(time::Duration::minutes(59)),
            now
        ));
        assert!(scan_due(
            ScanSchedule::Hourly,
            ago(time::Duration::HOUR),
            now
        ));
        assert!(!scan_due(
            ScanSchedule::Daily,
            ago(time::Duration::hours(23)),
            now
        ));
        assert!(scan_due(
            ScanSchedule::Daily,
            ago(time::Duration::hours(25)),
            now
        ));
        assert!(!scan_due(
            ScanSchedule::Weekly,
            ago(time::Duration::days(6)),
            now
        ));
        assert!(scan_due(
            ScanSchedule::Weekly,
            ago(time::Duration::days(8)),
            now
        ));
    }
}
