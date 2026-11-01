// SPDX-License-Identifier: AGPL-3.0-only
//
// In-memory capture of recent tracing events so admins can read
// warnings and errors without shell access. Two rings: a general one
// (INFO and up) that turns over quickly, and a WARN/ERROR one that
// survives bursts of INFO chatter. Scanner per-file failures are
// copied to a third ring the indexing view attributes to libraries.

use std::{
    collections::VecDeque,
    fmt::{self, Write as _},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use async_graphql::ID;
use noflex_graphql::admin_status::{LogEntry, LogField, LogFilter, LogLevel, LogPage, ScanFailure};
use time::OffsetDateTime;
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
    span,
};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

const RECENT_CAPACITY: usize = 2000;
const PROBLEM_CAPACITY: usize = 1000;
const SCAN_FAILURE_CAPACITY: usize = 1000;
/// Bounds one field so a stray multi-megabyte `?payload` can't pin the
/// ring's memory.
const MAX_VALUE_LEN: usize = 2048;

/// Bounded store of recent log events. Cheap to share; all methods
/// take `&self`.
pub struct LogBuffer {
    inner: Mutex<Rings>,
    next_seq: AtomicU64,
    warnings: AtomicU64,
    errors: AtomicU64,
    capacities: Capacities,
}

#[derive(Clone, Copy)]
struct Capacities {
    recent: usize,
    problems: usize,
    scan_failures: usize,
}

#[derive(Default)]
struct Rings {
    recent: VecDeque<LogEntry>,
    problems: VecDeque<LogEntry>,
    scan_failures: VecDeque<ScanFailure>,
    last_error: Option<LogEntry>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::with_capacities(RECENT_CAPACITY, PROBLEM_CAPACITY, SCAN_FAILURE_CAPACITY)
    }
}

impl LogBuffer {
    fn with_capacities(recent: usize, problems: usize, scan_failures: usize) -> Self {
        Self {
            inner: Mutex::new(Rings::default()),
            next_seq: AtomicU64::new(1),
            warnings: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            capacities: Capacities {
                recent,
                problems,
                scan_failures,
            },
        }
    }

    /// The `tracing_subscriber` layer feeding this buffer.
    pub fn layer(self: &Arc<Self>) -> LogCaptureLayer {
        LogCaptureLayer {
            buffer: self.clone(),
        }
    }

    // Poisoning only means a panic mid-push; the rings are still
    // structurally valid, so keep serving them.
    fn rings(&self) -> std::sync::MutexGuard<'_, Rings> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record one event. Entries below INFO are dropped.
    pub fn push(
        &self,
        at: OffsetDateTime,
        level: LogLevel,
        target: &str,
        message: String,
        fields: Vec<LogField>,
        spans: Vec<String>,
    ) {
        if level < LogLevel::Info {
            return;
        }
        match level {
            LogLevel::Warn => self.warnings.fetch_add(1, Ordering::Relaxed),
            LogLevel::Error => self.errors.fetch_add(1, Ordering::Relaxed),
            _ => 0,
        };
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let scan_failure = (level >= LogLevel::Warn && target.starts_with("noflex_scanner"))
            .then(|| scan_failure(at, &message, &fields))
            .flatten();
        let entry = LogEntry {
            id: ID(seq.to_string()),
            seq,
            at,
            level,
            target: target.to_owned(),
            message,
            fields,
            spans,
        };
        let caps = self.capacities;
        let mut rings = self.rings();
        if let Some(failure) = scan_failure {
            push_bounded(&mut rings.scan_failures, failure, caps.scan_failures);
        }
        if level >= LogLevel::Warn {
            if level == LogLevel::Error {
                rings.last_error = Some(entry.clone());
            }
            push_bounded(&mut rings.problems, entry.clone(), caps.problems);
        }
        push_bounded(&mut rings.recent, entry, caps.recent);
    }

    /// WARN and ERROR counts since startup, evicted entries included.
    pub fn counts(&self) -> (u64, u64) {
        (
            self.warnings.load(Ordering::Relaxed),
            self.errors.load(Ordering::Relaxed),
        )
    }

    pub fn last_error(&self) -> Option<LogEntry> {
        self.rings().last_error.clone()
    }

    /// Newest-first page over the union of both rings, so WARN/ERROR
    /// entries evicted from the general ring still appear once the walk
    /// passes its oldest entry.
    pub fn query(&self, filter: &LogFilter, before: Option<u64>, first: usize) -> LogPage {
        let needle = filter.contains.as_deref().map(str::to_lowercase);
        let matches = |e: &LogEntry| {
            before.is_none_or(|b| e.seq < b)
                && filter.min_level.is_none_or(|min| e.level >= min)
                && filter
                    .target_prefix
                    .as_deref()
                    .is_none_or(|p| e.target.starts_with(p))
                && needle.as_deref().is_none_or(|n| entry_contains(e, n))
        };
        let rings = self.rings();
        let problems_only = filter.min_level.is_some_and(|l| l >= LogLevel::Warn);
        let oldest_recent = rings.recent.front().map_or(u64::MAX, |e| e.seq);
        let general: Box<dyn Iterator<Item = &LogEntry>> = if problems_only {
            Box::new(std::iter::empty())
        } else {
            Box::new(rings.recent.iter().rev())
        };
        let older_problems = rings
            .problems
            .iter()
            .rev()
            .filter(move |e| problems_only || e.seq < oldest_recent);
        let mut hits = general.chain(older_problems).filter(|e| matches(e));
        let entries: Vec<LogEntry> = hits.by_ref().take(first).cloned().collect();
        let has_more = hits.next().is_some();
        LogPage { entries, has_more }
    }

    /// Retained scanner failures for files under any of `roots`, newest
    /// first: at most `cap` entries, plus the total match count.
    pub fn scan_failures_under(&self, roots: &[String], cap: usize) -> (Vec<ScanFailure>, u32) {
        let rings = self.rings();
        let mut hits = rings
            .scan_failures
            .iter()
            .rev()
            .filter(|f| under_any_root(&f.path, roots));
        let recent: Vec<ScanFailure> = hits.by_ref().take(cap).cloned().collect();
        let total = recent.len() + hits.count();
        (recent, u32::try_from(total).unwrap_or(u32::MAX))
    }

    /// Count of retained scanner failures under `roots` captured in
    /// `[from, to]`.
    pub fn scan_failures_between(
        &self,
        roots: &[String],
        from: OffsetDateTime,
        to: OffsetDateTime,
    ) -> u32 {
        let rings = self.rings();
        let n = rings
            .scan_failures
            .iter()
            .filter(|f| f.at >= from && f.at <= to && under_any_root(&f.path, roots))
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }
}

fn push_bounded<T>(ring: &mut VecDeque<T>, item: T, cap: usize) {
    if cap == 0 {
        return;
    }
    while ring.len() >= cap {
        ring.pop_front();
    }
    ring.push_back(item);
}

fn under_any_root(path: &str, roots: &[String]) -> bool {
    let path = Path::new(path);
    roots.iter().any(|r| path.starts_with(r))
}

fn entry_contains(e: &LogEntry, needle: &str) -> bool {
    e.message.to_lowercase().contains(needle)
        || e.fields
            .iter()
            .any(|f| f.value.to_lowercase().contains(needle))
}

/// Scanner warnings carrying a `path` field are per-file failures.
fn scan_failure(at: OffsetDateTime, message: &str, fields: &[LogField]) -> Option<ScanFailure> {
    let field = |name: &str| fields.iter().find(|f| f.name == name).map(|f| &f.value);
    let path = field("path")?.clone();
    Some(ScanFailure {
        at,
        path,
        stage: message.to_owned(),
        error: field("err").or_else(|| field("error")).cloned(),
    })
}

/// Feeds a [`LogBuffer`]. Install after the global filter so only
/// events the operator enabled are captured.
pub struct LogCaptureLayer {
    buffer: Arc<LogBuffer>,
}

/// Rendered `k=v` fields of a span, kept in its extensions so every
/// event inside it can name its context.
struct SpanFields(String);

impl<S> Layer<S> for LogCaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut fields = FieldCollector::default();
        attrs.record(&mut fields);
        span.extensions_mut()
            .insert(SpanFields(render_inline(&fields.fields)));
    }

    fn on_record(&self, id: &span::Id, values: &span::Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut fields = FieldCollector::default();
        values.record(&mut fields);
        if fields.fields.is_empty() {
            return;
        }
        let rendered = render_inline(&fields.fields);
        let mut ext = span.extensions_mut();
        match ext.get_mut::<SpanFields>() {
            Some(existing) if !existing.0.is_empty() => {
                existing.0.push(' ');
                existing.0.push_str(&rendered);
            }
            Some(existing) => existing.0 = rendered,
            None => ext.insert(SpanFields(rendered)),
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let meta = event.metadata();
        let level = LogLevel::from(*meta.level());
        if level < LogLevel::Info {
            return;
        }
        let mut collector = FieldCollector::default();
        event.record(&mut collector);
        let spans = ctx
            .event_scope(event)
            .map(|scope| {
                scope
                    .from_root()
                    .map(|span| {
                        let ext = span.extensions();
                        match ext.get::<SpanFields>() {
                            Some(f) if !f.0.is_empty() => format!("{}{{{}}}", span.name(), f.0),
                            _ => span.name().to_owned(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.buffer.push(
            OffsetDateTime::now_utc(),
            level,
            meta.target(),
            collector.message.unwrap_or_default(),
            collector.fields,
            spans,
        );
    }
}

#[derive(Default)]
struct FieldCollector {
    message: Option<String>,
    fields: Vec<LogField>,
}

impl FieldCollector {
    fn put(&mut self, field: &Field, value: String) {
        let value = truncate(value);
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push(LogField {
                name: field.name().to_owned(),
                value,
            });
        }
    }
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_owned());
    }

    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        let mut rendered = value.to_string();
        let mut source = value.source();
        while let Some(cause) = source {
            let _ = write!(rendered, ": {cause}");
            source = cause.source();
        }
        self.put(field, rendered);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, unquote(format!("{value:?}")));
    }
}

/// `?path` on a `PathBuf`/`String` debug-formats with quotes; strip
/// them so failure paths compare against library roots.
fn unquote(s: String) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        s
    }
}

fn truncate(mut s: String) -> String {
    if s.len() > MAX_VALUE_LEN {
        let mut cut = MAX_VALUE_LEN;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

fn render_inline(fields: &[LogField]) -> String {
    let mut out = String::new();
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{}={}", f.name, f.value);
    }
    out
}

#[cfg(test)]
mod tests {
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    fn push(buf: &LogBuffer, level: LogLevel, target: &str, msg: &str) {
        buf.push(
            OffsetDateTime::now_utc(),
            level,
            target,
            msg.to_owned(),
            Vec::new(),
            Vec::new(),
        );
    }

    fn messages(page: &LogPage) -> Vec<&str> {
        page.entries.iter().map(|e| e.message.as_str()).collect()
    }

    #[test]
    fn rings_are_bounded_and_newest_first() {
        let buf = LogBuffer::with_capacities(3, 2, 2);
        for i in 0..5 {
            push(&buf, LogLevel::Info, "t", &format!("m{i}"));
        }
        let page = buf.query(&LogFilter::default(), None, 10);
        assert_eq!(messages(&page), ["m4", "m3", "m2"]);
        assert!(!page.has_more);
    }

    #[test]
    fn debug_and_trace_are_dropped() {
        let buf = LogBuffer::default();
        push(&buf, LogLevel::Debug, "t", "noise");
        push(&buf, LogLevel::Trace, "t", "noise");
        assert!(
            buf.query(&LogFilter::default(), None, 10)
                .entries
                .is_empty()
        );
    }

    #[test]
    fn warnings_outlive_info_churn_and_merge_in_order() {
        let buf = LogBuffer::with_capacities(2, 10, 1);
        push(&buf, LogLevel::Warn, "t", "w1");
        push(&buf, LogLevel::Error, "t", "e1");
        for i in 0..4 {
            push(&buf, LogLevel::Info, "t", &format!("i{i}"));
        }
        let all = buf.query(&LogFilter::default(), None, 10);
        assert_eq!(messages(&all), ["i3", "i2", "e1", "w1"]);

        let problems = buf.query(
            &LogFilter {
                min_level: Some(LogLevel::Error),
                ..Default::default()
            },
            None,
            10,
        );
        assert_eq!(messages(&problems), ["e1"]);
        assert_eq!(buf.counts(), (1, 1));
        assert_eq!(buf.last_error().map(|e| e.message), Some("e1".into()));
    }

    #[test]
    fn cursor_pages_without_overlap() {
        let buf = LogBuffer::default();
        for i in 0..5 {
            push(&buf, LogLevel::Info, "t", &format!("m{i}"));
        }
        let first = buf.query(&LogFilter::default(), None, 2);
        assert_eq!(messages(&first), ["m4", "m3"]);
        assert!(first.has_more);
        let cursor = first.entries.last().map(|e| e.seq);
        let second = buf.query(&LogFilter::default(), cursor, 2);
        assert_eq!(messages(&second), ["m2", "m1"]);
        let cursor = second.entries.last().map(|e| e.seq);
        let third = buf.query(&LogFilter::default(), cursor, 2);
        assert_eq!(messages(&third), ["m0"]);
        assert!(!third.has_more);
    }

    #[test]
    fn filters_by_target_and_text() {
        let buf = LogBuffer::default();
        push(&buf, LogLevel::Info, "noflex_scanner::scan", "Scan Started");
        push(
            &buf,
            LogLevel::Info,
            "noflex_server::stream",
            "stream started",
        );
        let by_target = buf.query(
            &LogFilter {
                target_prefix: Some("noflex_scanner".into()),
                ..Default::default()
            },
            None,
            10,
        );
        assert_eq!(messages(&by_target), ["Scan Started"]);
        let by_text = buf.query(
            &LogFilter {
                contains: Some("scan start".into()),
                ..Default::default()
            },
            None,
            10,
        );
        assert_eq!(messages(&by_text), ["Scan Started"]);
    }

    #[test]
    fn layer_captures_fields_spans_and_scan_failures() {
        let buf = Arc::new(LogBuffer::default());
        let subscriber = tracing_subscriber::registry().with(buf.layer());
        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("scan", library = "lib-1");
            let _guard = span.enter();
            tracing::warn!(
                target: "noflex_scanner::scan",
                path = ?std::path::PathBuf::from("/media/movies/a.mkv"),
                err = "boom",
                "scan: file errored"
            );
            tracing::debug!("dropped");
        });
        let page = buf.query(&LogFilter::default(), None, 10);
        assert_eq!(page.entries.len(), 1);
        let entry = &page.entries[0];
        assert_eq!(entry.level, LogLevel::Warn);
        assert_eq!(entry.message, "scan: file errored");
        assert_eq!(entry.spans, ["scan{library=lib-1}"]);
        assert!(
            entry
                .fields
                .iter()
                .any(|f| f.name == "path" && f.value == "/media/movies/a.mkv")
        );

        let (failures, total) = buf.scan_failures_under(&["/media/movies".into()], 10);
        assert_eq!(total, 1);
        assert_eq!(failures[0].error.as_deref(), Some("boom"));
        assert_eq!(buf.scan_failures_under(&["/media/tv".into()], 10).1, 0);
        // Component-wise, not textual, prefix matching.
        assert_eq!(buf.scan_failures_under(&["/media/mov".into()], 10).1, 0);
    }

    #[test]
    fn long_values_are_truncated_on_char_boundaries() {
        let s = "é".repeat(MAX_VALUE_LEN);
        let t = truncate(s);
        assert!(t.len() <= MAX_VALUE_LEN + '…'.len_utf8());
        assert!(t.ends_with('…'));
    }
}
