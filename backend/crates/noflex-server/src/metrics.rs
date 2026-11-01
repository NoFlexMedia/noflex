// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use axum::{
    body::Body,
    http::{StatusCode, header},
    response::Response,
};
use serde::Serialize;

/// Per-process counters used by the /metrics exporter. Counters are
/// monotonic, never reset; the Prometheus scrape itself records the
/// "since server start" baseline. Real production code uses
/// `metrics` + `metrics-exporter-prometheus`; the in-process implementation
/// is a hand-written text format because it's testable, has
/// no extra deps, and the format is small.
#[derive(Debug, Default)]
pub struct Metrics {
    pub requests_total: AtomicU64,
    pub auth_success_total: AtomicU64,
    pub auth_failure_total: AtomicU64,
    pub scans_completed_total: AtomicU64,
    pub scans_failed_total: AtomicU64,
    pub transcode_jobs_started_total: AtomicU64,
    pub transcode_jobs_completed_total: AtomicU64,
    /// Latest host/process reading, published by the admin status
    /// sampler. `None` until the first sample (or when no sampler runs).
    gauges: Mutex<Option<HostGauges>>,
}

/// Point-in-time gauges mirrored from the admin status sampler so
/// Prometheus scrapes see the same figures as the admin UI.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HostGauges {
    pub host_cpu_percent: f64,
    pub host_memory_used_bytes: u64,
    pub host_memory_total_bytes: u64,
    pub process_cpu_percent: f64,
    pub process_rss_bytes: u64,
    pub log_warnings_total: u64,
    pub log_errors_total: u64,
    pub active_scans: u64,
}

#[allow(dead_code)]
impl Metrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn inc_requests(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_auth_success(&self) {
        self.auth_success_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_auth_failure(&self) {
        self.auth_failure_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_scan_completed(&self) {
        self.scans_completed_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_scan_failed(&self) {
        self.scans_failed_total.fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_transcode_started(&self) {
        self.transcode_jobs_started_total
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn inc_transcode_completed(&self) {
        self.transcode_jobs_completed_total
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_host_gauges(&self, gauges: HostGauges) {
        *self.gauges.lock().unwrap_or_else(|e| e.into_inner()) = Some(gauges);
    }

    pub fn host_gauges(&self) -> Option<HostGauges> {
        *self.gauges.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[derive(Serialize)]
struct Counter<'a> {
    name: &'a str,
    help: &'a str,
    value: u64,
}

pub fn render(m: &Metrics) -> String {
    let counters = [
        Counter {
            name: "noflex_requests_total",
            help: "Total HTTP requests handled by the server.",
            value: m.requests_total.load(Ordering::Relaxed),
        },
        Counter {
            name: "noflex_auth_success_total",
            help: "Total successful auth checks (token valid).",
            value: m.auth_success_total.load(Ordering::Relaxed),
        },
        Counter {
            name: "noflex_auth_failure_total",
            help: "Total failed auth checks (token missing/invalid/expired).",
            value: m.auth_failure_total.load(Ordering::Relaxed),
        },
        Counter {
            name: "noflex_scans_completed_total",
            help: "Total library scans that completed without error.",
            value: m.scans_completed_total.load(Ordering::Relaxed),
        },
        Counter {
            name: "noflex_scans_failed_total",
            help: "Total library scans that failed.",
            value: m.scans_failed_total.load(Ordering::Relaxed),
        },
        Counter {
            name: "noflex_transcode_jobs_started_total",
            help: "Total transcode jobs started.",
            value: m.transcode_jobs_started_total.load(Ordering::Relaxed),
        },
        Counter {
            name: "noflex_transcode_jobs_completed_total",
            help: "Total transcode jobs that finished (success or failure).",
            value: m.transcode_jobs_completed_total.load(Ordering::Relaxed),
        },
    ];
    let mut out = String::with_capacity(counters.len() * 96);
    for c in &counters {
        out.push_str(&format!(
            "# HELP {} {}\n# TYPE {} counter\n{} {}\n",
            c.name, c.help, c.name, c.name, c.value
        ));
    }
    if let Some(g) = m.host_gauges() {
        let gauges: [(&str, &str, &str, String); 8] = [
            (
                "noflex_host_cpu_usage_percent",
                "Whole-host CPU utilisation, 0-100.",
                "gauge",
                g.host_cpu_percent.to_string(),
            ),
            (
                "noflex_host_memory_used_bytes",
                "Host memory in use.",
                "gauge",
                g.host_memory_used_bytes.to_string(),
            ),
            (
                "noflex_host_memory_total_bytes",
                "Host memory installed.",
                "gauge",
                g.host_memory_total_bytes.to_string(),
            ),
            (
                "noflex_process_cpu_percent",
                "Server process CPU, summed over cores (100 = one core).",
                "gauge",
                g.process_cpu_percent.to_string(),
            ),
            (
                "noflex_process_resident_memory_bytes",
                "Server process resident set size.",
                "gauge",
                g.process_rss_bytes.to_string(),
            ),
            (
                "noflex_log_warnings_total",
                "WARN events logged since server start.",
                "counter",
                g.log_warnings_total.to_string(),
            ),
            (
                "noflex_log_errors_total",
                "ERROR events logged since server start.",
                "counter",
                g.log_errors_total.to_string(),
            ),
            (
                "noflex_active_scans",
                "Libraries with a scan in flight.",
                "gauge",
                g.active_scans.to_string(),
            ),
        ];
        for (name, help, kind, value) in &gauges {
            out.push_str(&format!(
                "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}\n"
            ));
        }
    }
    out
}

pub async fn render_response(metrics: Arc<Metrics>) -> Response {
    let mut response = Response::new(Body::from(render(&metrics)));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/plain; version=0.0.4"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_includes_all_counters_in_prometheus_text_format() {
        let m = Metrics::new();
        m.inc_requests();
        m.inc_requests();
        m.inc_auth_success();
        m.inc_auth_failure();
        m.inc_scan_completed();
        m.inc_scan_failed();
        m.inc_transcode_started();
        m.inc_transcode_completed();
        let out = render(&m);
        assert!(out.contains("noflex_requests_total 2"));
        assert!(out.contains("noflex_auth_success_total 1"));
        assert!(out.contains("noflex_auth_failure_total 1"));
        assert!(out.contains("noflex_scans_completed_total 1"));
        assert!(out.contains("noflex_scans_failed_total 1"));
        assert!(out.contains("noflex_transcode_jobs_started_total 1"));
        assert!(out.contains("noflex_transcode_jobs_completed_total 1"));
    }

    #[test]
    fn render_includes_host_gauges_once_published() {
        let m = Metrics::new();
        assert!(!render(&m).contains("noflex_process_resident_memory_bytes"));
        m.set_host_gauges(HostGauges {
            process_rss_bytes: 4096,
            log_errors_total: 3,
            ..HostGauges::default()
        });
        let out = render(&m);
        assert!(out.contains("# TYPE noflex_process_resident_memory_bytes gauge"));
        assert!(out.contains("noflex_process_resident_memory_bytes 4096"));
        assert!(out.contains("noflex_log_errors_total 3"));
    }

    #[test]
    fn render_handles_zero_state() {
        let m = Metrics::new();
        let out = render(&m);
        assert!(out.contains("noflex_requests_total 0"));
        assert!(out.contains("# TYPE noflex_requests_total counter"));
    }
}
