// SPDX-License-Identifier: AGPL-3.0-only
//
// Audit-log target conventions for the plugin host. All capability uses
// (HTTP egress, FS read, secret read, host-library call) emit on the
// `noflex_plugin::audit` tracing target. Operators wire a dedicated
// `tracing-subscriber` layer to route this target to a separate appender.
//
// This module defines the convention and provides typed helpers so call
// sites can't accidentally use the wrong field names.

/// The tracing target every audit event uses.
pub const TARGET: &str = "noflex_plugin::audit";

/// Helper: log one HTTP egress attempt (allowed or denied) to the audit
/// appender. `status` is the upstream status, `Some(0)` for a host-side
/// denial, or `None` when the request failed in transit.
///
/// The URL is never logged verbatim: webhook URLs (Discord, Slack, ...)
/// carry credentials in their path or query. Only the origin plus a short
/// digest of the path and query is recorded — enough to correlate repeated
/// calls to the same endpoint without leaking the secret.
pub fn http_egress(plugin_id: &str, method: &str, url: &str, status: Option<u16>) {
    let destination = redact_url(url);
    match status {
        Some(s) => tracing::info!(
            target: TARGET,
            plugin = plugin_id,
            event = "http_egress",
            method,
            destination = %destination,
            status = s,
        ),
        None => tracing::info!(
            target: TARGET,
            plugin = plugin_id,
            event = "http_egress",
            method,
            destination = %destination,
        ),
    }
}

/// `scheme://host[:port] #<digest>`, where the digest is the first 12 hex
/// digits of SHA-256 over the path and query (over the whole input when it
/// does not parse). Userinfo and fragments are dropped.
pub(crate) fn redact_url(raw: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = |input: &str| {
        let hash = Sha256::digest(input.as_bytes());
        hash.iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    match url::Url::parse(raw) {
        Ok(url) => {
            let host = match url.host() {
                Some(host) => host.to_string(),
                None => String::new(),
            };
            let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
            let rest = match url.query() {
                Some(query) => format!("{}?{query}", url.path()),
                None => url.path().to_string(),
            };
            format!("{}://{host}{port} #{}", url.scheme(), digest(&rest))
        }
        Err(_) => format!("<unparsable> #{}", digest(raw)),
    }
}

/// Helper: log a filesystem read. `path` is the absolute path after the
/// host's WASI preopen resolution; this is what was actually opened, not
/// the guest's view.
pub fn fs_read(plugin_id: &str, path: &str) {
    tracing::info!(
        target: TARGET,
        plugin = plugin_id,
        event = "fs_read",
        path,
    );
}

/// Helper: log a secret read. The secret *value* is never logged; only
/// the key name and whether the read was permitted.
pub fn secret_read(plugin_id: &str, key: &str, granted: bool) {
    tracing::info!(
        target: TARGET,
        plugin = plugin_id,
        event = "secret_read",
        key,
        granted,
    );
}

/// Helper: log a per-user secret read (`host-secrets.read-for-user`).
/// The value is never logged; the user id, key, and whether the read
/// was permitted are.
pub fn user_secret_read(plugin_id: &str, user: &str, key: &str, granted: bool) {
    tracing::info!(
        target: TARGET,
        plugin = plugin_id,
        event = "user_secret_read",
        user,
        key,
        granted,
    );
}

/// Helper: log a per-user secret write after a completed link flow.
/// The value is never logged; the plugin, user, and key are.
pub fn user_secret_write(plugin_id: &str, user: &str, key: &str) {
    tracing::info!(
        target: TARGET,
        plugin = plugin_id,
        event = "user_secret_write",
        user,
        key,
    );
}

/// Helper: log a host-library lookup.
pub fn library_lookup(plugin_id: &str, op: &str, key: &str, hit: bool) {
    tracing::info!(
        target: TARGET,
        plugin = plugin_id,
        event = "library_lookup",
        op,
        key,
        hit,
    );
}

/// Helper: log a `host-tasks.enqueue` call. The payload is never logged;
/// only the task name and whether the enqueue was accepted.
pub fn task_enqueue(plugin_id: &str, task: &str, accepted: bool) {
    tracing::info!(
        target: TARGET,
        plugin = plugin_id,
        event = "task_enqueue",
        task,
        accepted,
    );
}

/// Helper: log a `host-assets.read-file` call. `bytes` is the served
/// size on success; `None` means the read was denied or failed.
pub fn asset_read(plugin_id: &str, name: &str, bytes: Option<u64>) {
    tracing::info!(
        target: TARGET,
        plugin = plugin_id,
        event = "asset_read",
        name,
        bytes = bytes.unwrap_or(0),
        granted = bytes.is_some(),
    );
}

/// Helper: log a `host-inference` run over `sequences` token sequences.
pub fn inference_run(plugin_id: &str, model: &str, sequences: u64, ok: bool) {
    tracing::info!(
        target: TARGET,
        plugin = plugin_id,
        event = "inference_run",
        model,
        sequences,
        ok,
    );
}

/// Helper: log a background-task invocation boundary. Fires on start and
/// end only — not per fire — so a 50ms-interval task doesn't drown the
/// audit appender.
pub fn task_run(plugin_id: &str, task: &str, phase: &str, detail: &str) {
    tracing::info!(
        target: TARGET,
        plugin = plugin_id,
        event = "task_run",
        task,
        phase,
        detail,
    );
}

#[cfg(test)]
mod redact_tests {
    use super::redact_url;

    #[test]
    fn webhook_tokens_never_reach_the_audit_log() {
        let url = "https://discord.com/api/webhooks/123/s3cr3t-token?wait=true";
        let redacted = redact_url(url);
        assert!(redacted.starts_with("https://discord.com #"), "{redacted}");
        assert!(!redacted.contains("s3cr3t"));
        assert!(!redacted.contains("wait"));
        assert_eq!(redacted, redact_url(url), "digest is stable");
        assert_ne!(
            redacted,
            redact_url("https://discord.com/api/webhooks/123/other")
        );
    }

    #[test]
    fn origin_keeps_explicit_ports_and_drops_userinfo() {
        let redacted = redact_url("http://user:pw@[::1]:8080/hook#frag");
        assert!(redacted.starts_with("http://[::1]:8080 #"), "{redacted}");
        assert!(!redacted.contains("pw"));
        assert!(redact_url("not a url with token").starts_with("<unparsable> #"));
    }
}
