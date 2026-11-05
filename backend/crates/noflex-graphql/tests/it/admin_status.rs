// SPDX-License-Identifier: AGPL-3.0-only
//
// Admin diagnostics surface: every query and the live subscription are
// administrator-only, connections page with `first`/`after`, and the
// library index view composes storage, the scan registry and the
// service's per-library activity.

use std::sync::Arc;

use async_graphql::{ID, Request};
use futures::StreamExt;
use noflex_graphql::{
    AdminStatusError, AdminStatusService, SchemaContext,
    admin_status::{
        IndexScanRun, LibraryActivity, LibraryWatcherStatus, LogEntry, LogFilter, LogLevel,
        LogPage, PluginEventPage, SearchIndexStatus, StatusSample, SystemStatus, TranscoderInfo,
        WatcherState,
    },
    build_schema,
};
use noflex_storage::{RequestUser, Storage, domain::Library};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;
use tokio::sync::watch;

struct StubService {
    tx: watch::Sender<Arc<SystemStatus>>,
    logs: Vec<LogEntry>,
}

fn status(cpu: f32) -> SystemStatus {
    let now = OffsetDateTime::now_utc();
    SystemStatus {
        sampled_at: now,
        server: noflex_graphql::admin_status::ServerRuntime {
            version: "0.0.1".into(),
            started_at: now,
            uptime_seconds: 5,
            bind: "127.0.0.1:8099".into(),
            public_url: None,
            data_dir: "/data".into(),
            cache_dir: "/cache".into(),
            config: Vec::new(),
            database: Default::default(),
        },
        host: Default::default(),
        cpu: noflex_graphql::admin_status::CpuStatus {
            usage_percent: cpu,
            ..Default::default()
        },
        memory: Default::default(),
        process: Default::default(),
        network: Vec::new(),
        network_total: Default::default(),
        storage: Vec::new(),
        plugins: Default::default(),
        logs: Default::default(),
        active_scans: 0,
    }
}

fn log(seq: u64, level: LogLevel, message: &str) -> LogEntry {
    LogEntry {
        id: ID(seq.to_string()),
        seq,
        at: OffsetDateTime::now_utc(),
        level,
        target: "noflex_test".into(),
        message: message.into(),
        fields: Vec::new(),
        spans: Vec::new(),
    }
}

#[async_trait::async_trait]
impl AdminStatusService for StubService {
    fn system_status(&self) -> Arc<SystemStatus> {
        self.tx.borrow().clone()
    }
    fn status_history(&self) -> Vec<StatusSample> {
        Vec::new()
    }
    fn watch_system_status(&self) -> watch::Receiver<Arc<SystemStatus>> {
        self.tx.subscribe()
    }
    async fn transcoder_info(&self) -> Arc<TranscoderInfo> {
        Arc::new(TranscoderInfo {
            ffmpeg: Default::default(),
            ffprobe: Default::default(),
            configured_hwaccel: "auto".into(),
            fallback: "auto".into(),
            resolved_backend: "software".into(),
            hardware_accelerated: false,
            fell_back: false,
            hwaccels: Vec::new(),
            vaapi_device: None,
            gpu_tonemap: false,
            encoders: Vec::new(),
            decoders: Vec::new(),
            ladder: Vec::new(),
            transcode_dir: "/t".into(),
        })
    }
    async fn library_activity(&self, _library: &Library) -> LibraryActivity {
        LibraryActivity {
            last_scan: Some(IndexScanRun {
                added: 7,
                ..Default::default()
            }),
            watcher: LibraryWatcherStatus {
                state: WatcherState::Active,
                ..Default::default()
            },
            ..Default::default()
        }
    }
    async fn search_index_status(&self) -> Result<SearchIndexStatus, AdminStatusError> {
        Ok(SearchIndexStatus::default())
    }
    fn recent_logs(&self, filter: &LogFilter, before: Option<u64>, first: usize) -> LogPage {
        let mut hits = self
            .logs
            .iter()
            .rev()
            .filter(|e| before.is_none_or(|b| e.seq < b))
            .filter(|e| filter.min_level.is_none_or(|m| e.level >= m));
        let entries: Vec<LogEntry> = hits.by_ref().take(first).cloned().collect();
        LogPage {
            entries,
            has_more: hits.next().is_some(),
        }
    }
    fn plugin_events(&self, _before: Option<u64>, _first: usize) -> PluginEventPage {
        PluginEventPage::default()
    }
}

async fn schema() -> (
    noflex_graphql::NoflexSchema,
    watch::Sender<Arc<SystemStatus>>,
) {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    store
        .upsert_library(&Library {
            id: noflex_core::LibraryId::new(),
            name: "Films".into(),
            kind: noflex_storage::domain::LibraryKind::Movies,
            paths: vec!["/srv/films".into()],
            ..Default::default()
        })
        .await
        .expect("library");
    let storage: Arc<dyn Storage> = Arc::new(store);
    let (tx, _) = watch::channel(Arc::new(status(12.5)));
    let service = Arc::new(StubService {
        tx: tx.clone(),
        logs: vec![
            log(1, LogLevel::Info, "booted"),
            log(2, LogLevel::Warn, "slow disk"),
            log(3, LogLevel::Error, "ffmpeg crashed"),
        ],
    });
    let schema = build_schema(SchemaContext {
        admin_status: Some(service),
        ..SchemaContext::new(storage)
    });
    (schema, tx)
}

fn as_user(query: &str, is_admin: bool) -> Request {
    Request::new(query).data(RequestUser {
        user_id: noflex_core::UserId::new(),
        is_admin,
    })
}

const ADMIN_QUERIES: &[&str] = &[
    "{ systemStatus { sampledAt } }",
    "{ systemStatusHistory { at } }",
    "{ transcoderInfo { resolvedBackend } }",
    "{ scannerStatus { failureCount } }",
    "{ searchIndexStatus { pendingWorks } }",
    "{ recentLogs { edges { cursor } } }",
    "{ pluginEvents { edges { cursor } } }",
];

#[tokio::test]
async fn non_admins_and_anonymous_callers_are_rejected() {
    let (schema, _tx) = schema().await;
    for query in ADMIN_QUERIES {
        let resp = schema.execute(as_user(query, false)).await;
        assert_eq!(
            resp.errors.first().map(|e| e.message.as_str()),
            Some("administrator required"),
            "{query} must be admin-only"
        );
        let resp = schema.execute(Request::new(*query)).await;
        assert!(!resp.errors.is_empty(), "{query} must require auth");
    }

    let mut stream = schema.execute_stream(as_user(
        "subscription { systemStatusUpdates { sampledAt } }",
        false,
    ));
    let first = stream.next().await.expect("one frame");
    assert_eq!(
        first.errors.first().map(|e| e.message.as_str()),
        Some("administrator required")
    );
}

#[tokio::test]
async fn admins_read_every_section() {
    let (schema, _tx) = schema().await;
    for query in ADMIN_QUERIES {
        let resp = schema.execute(as_user(query, true)).await;
        assert!(resp.errors.is_empty(), "{query}: {:?}", resp.errors);
    }
    let resp = schema
        .execute(as_user(
            "{ systemStatus { cpu { usagePercent } server { bind } } \
               scannerStatus { library { name } scan { state } watcher { state } lastScan { added } } }",
            true,
        ))
        .await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    let data = resp.data.into_json().expect("json");
    assert_eq!(data["systemStatus"]["cpu"]["usagePercent"], 12.5);
    assert_eq!(data["systemStatus"]["server"]["bind"], "127.0.0.1:8099");
    let lib = &data["scannerStatus"][0];
    assert_eq!(lib["library"]["name"], "Films");
    assert_eq!(lib["scan"]["state"], "IDLE");
    assert_eq!(lib["watcher"]["state"], "ACTIVE");
    assert_eq!(lib["lastScan"]["added"], 7);
}

#[tokio::test]
async fn recent_logs_page_with_first_and_after() {
    let (schema, _tx) = schema().await;
    let page = |after: Option<&str>| {
        let after = after.map_or(String::new(), |a| format!(", after: \"{a}\""));
        format!(
            "{{ recentLogs(first: 2{after}) {{ edges {{ cursor node {{ message level }} }} \
               pageInfo {{ hasNextPage endCursor }} }} }}"
        )
    };
    let resp = schema.execute(as_user(&page(None), true)).await;
    assert!(resp.errors.is_empty(), "{:?}", resp.errors);
    let data = resp.data.into_json().expect("json");
    let conn = &data["recentLogs"];
    assert_eq!(conn["edges"][0]["node"]["message"], "ffmpeg crashed");
    assert_eq!(conn["edges"][0]["node"]["level"], "ERROR");
    assert_eq!(conn["edges"].as_array().map(Vec::len), Some(2));
    assert_eq!(conn["pageInfo"]["hasNextPage"], true);
    let end = conn["pageInfo"]["endCursor"]
        .as_str()
        .expect("cursor")
        .to_owned();

    let resp = schema.execute(as_user(&page(Some(&end)), true)).await;
    let data = resp.data.into_json().expect("json");
    let conn = &data["recentLogs"];
    assert_eq!(conn["edges"].as_array().map(Vec::len), Some(1));
    assert_eq!(conn["edges"][0]["node"]["message"], "booted");
    assert_eq!(conn["pageInfo"]["hasNextPage"], false);

    let resp = schema
        .execute(as_user(
            "{ recentLogs(minLevel: WARN) { edges { node { message } } } }",
            true,
        ))
        .await;
    let data = resp.data.into_json().expect("json");
    assert_eq!(
        data["recentLogs"]["edges"].as_array().map(Vec::len),
        Some(2)
    );

    let resp = schema
        .execute(as_user(
            "{ recentLogs(after: \"nope\") { edges { cursor } } }",
            true,
        ))
        .await;
    assert!(!resp.errors.is_empty(), "malformed cursor must error");
}

#[tokio::test]
async fn subscription_yields_current_then_each_new_sample() {
    let (schema, tx) = schema().await;
    let mut stream = schema.execute_stream(as_user(
        "subscription { systemStatusUpdates { cpu { usagePercent } } }",
        true,
    ));
    let first = stream.next().await.expect("first frame");
    assert!(first.errors.is_empty(), "{:?}", first.errors);
    let first = first.data.into_json().expect("json");
    assert_eq!(first["systemStatusUpdates"]["cpu"]["usagePercent"], 12.5);

    tx.send_replace(Arc::new(status(50.0)));
    let next = stream.next().await.expect("second frame");
    let next = next.data.into_json().expect("json");
    assert_eq!(next["systemStatusUpdates"]["cpu"]["usagePercent"], 50.0);
}

#[tokio::test]
async fn missing_service_reports_unconfigured() {
    let store = TursoStore::open_temp().await.expect("temp store");
    store.migrate().await.expect("migrate");
    let schema = build_schema(SchemaContext::new(Arc::new(store)));
    let resp = schema
        .execute(as_user("{ systemStatus { sampledAt } }", true))
        .await;
    assert_eq!(
        resp.errors.first().map(|e| e.message.as_str()),
        Some("admin status not configured")
    );
}
