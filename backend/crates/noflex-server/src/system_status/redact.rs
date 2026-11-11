// SPDX-License-Identifier: AGPL-3.0-only
//
// Flatten the resolved config into dotted key/value rows for the admin
// view, masking anything that looks like a credential. Admin pages get
// screenshotted into bug reports, so the bar is "no secret leaves the
// box", not "the admin may not see it".

use noflex_graphql::admin_status::ConfigEntry;
use serde_json::Value;

const MASK: &str = "********";

/// Key fragments that mark a value as secret (matched against the
/// lowercased leaf key).
const SECRET_FRAGMENTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "api_key",
    "credential",
    "private",
    "signing",
    "cookie",
    "session_key",
];

/// Flatten `config` (any serde-serializable tree) into sorted rows.
/// Objects nest with `.`, arrays with `[i]`; empty containers yield one
/// row with no value so the key stays visible.
pub fn flatten_config(config: &Value) -> Vec<ConfigEntry> {
    let mut out = Vec::new();
    walk("", None, config, &mut out);
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

fn walk(path: &str, leaf_key: Option<&str>, value: &Value, out: &mut Vec<ConfigEntry>) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            for (k, v) in map {
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                walk(&child, Some(k), v, out);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for (i, v) in items.iter().enumerate() {
                walk(&format!("{path}[{i}]"), leaf_key, v, out);
            }
        }
        _ => out.push(leaf(path, leaf_key, value)),
    }
}

fn leaf(path: &str, leaf_key: Option<&str>, value: &Value) -> ConfigEntry {
    let rendered = match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        Value::Object(_) => Some("{}".to_owned()),
        Value::Array(_) => Some("[]".to_owned()),
        other => Some(other.to_string()),
    };
    let secret_key = leaf_key.is_some_and(is_secret_key);
    match rendered {
        Some(v) if secret_key && !v.is_empty() => ConfigEntry {
            key: path.to_owned(),
            value: Some(MASK.to_owned()),
            redacted: true,
        },
        Some(v) => {
            let (value, redacted) = redact_url(&v);
            ConfigEntry {
                key: path.to_owned(),
                value: Some(value),
                redacted,
            }
        }
        None => ConfigEntry {
            key: path.to_owned(),
            value: None,
            redacted: false,
        },
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key == "key"
        || key.ends_with("_key")
        || key.ends_with("-key")
        || SECRET_FRAGMENTS.iter().any(|f| key.contains(f))
}

/// Mask userinfo and secret-named query parameters inside URL values;
/// non-URLs pass through untouched.
fn redact_url(value: &str) -> (String, bool) {
    let Ok(mut url) = url::Url::parse(value) else {
        return (value.to_owned(), false);
    };
    if url.cannot_be_a_base() {
        return (value.to_owned(), false);
    }
    let mut redacted = false;
    if url.password().is_some() {
        let _ = url.set_password(Some(MASK));
        redacted = true;
    }
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if pairs.iter().any(|(k, _)| is_secret_key(k)) {
        let mut q = url.query_pairs_mut();
        q.clear();
        for (k, v) in &pairs {
            q.append_pair(k, if is_secret_key(k) { MASK } else { v });
        }
        drop(q);
        redacted = true;
    }
    if redacted {
        (url.to_string(), true)
    } else {
        (value.to_owned(), false)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn get<'a>(rows: &'a [ConfigEntry], key: &str) -> &'a ConfigEntry {
        rows.iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("no row {key}"))
    }

    #[test]
    fn flattens_nested_objects_and_arrays() {
        let rows = flatten_config(&json!({
            "bind": "0.0.0.0:28793",
            "transcoder": { "ladder": [{ "max_pixels": 2073600 }], "hwaccel": "auto" },
            "public_url": null,
            "plugin": { "search_path": [] },
        }));
        assert_eq!(get(&rows, "bind").value.as_deref(), Some("0.0.0.0:28793"));
        assert_eq!(
            get(&rows, "transcoder.ladder[0].max_pixels")
                .value
                .as_deref(),
            Some("2073600")
        );
        assert_eq!(get(&rows, "public_url").value, None);
        assert_eq!(
            get(&rows, "plugin.search_path").value.as_deref(),
            Some("[]")
        );
        let keys: Vec<_> = rows.iter().map(|r| r.key.as_str()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn masks_secret_named_keys() {
        let rows = flatten_config(&json!({
            "plugin": { "per_plugin_config": {
                "tmdb": { "api_key": "abc", "language": "en" },
                "trakt": { "client_secret": "s3", "client_id": "public" },
                "x": { "Token": "t", "key": "k", "tokens": ["a", "b"], "password": "" },
            }},
            "external_auth": { "auto_provision": true },
        }));
        let p = "plugin.per_plugin_config";
        for key in [
            format!("{p}.tmdb.api_key"),
            format!("{p}.trakt.client_secret"),
            format!("{p}.x.Token"),
            format!("{p}.x.key"),
            format!("{p}.x.tokens[0]"),
        ] {
            let row = get(&rows, &key);
            assert!(row.redacted, "{key} not redacted");
            assert_eq!(row.value.as_deref(), Some(MASK));
        }
        assert_eq!(
            get(&rows, &format!("{p}.tmdb.language")).value.as_deref(),
            Some("en")
        );
        assert!(!get(&rows, &format!("{p}.trakt.client_id")).redacted);
        // Empty secrets reveal nothing; showing them as unset helps debugging.
        assert!(!get(&rows, &format!("{p}.x.password")).redacted);
        assert!(!get(&rows, "external_auth.auto_provision").redacted);
    }

    #[test]
    fn masks_url_credentials_and_secret_query_params() {
        let rows = flatten_config(&json!({
            "a": "http://user:hunter2@host:8080/path",
            "b": "https://host/x?api_key=abc&lang=en",
            "c": "https://media.example",
            "d": "/not/a/url",
        }));
        let a = get(&rows, "a");
        assert!(a.redacted);
        assert!(!a.value.as_deref().unwrap_or_default().contains("hunter2"));
        assert!(a.value.as_deref().unwrap_or_default().contains("user"));
        let b = get(&rows, "b");
        assert!(b.redacted);
        let b = b.value.as_deref().unwrap_or_default();
        assert!(!b.contains("abc") && b.contains("lang=en"));
        assert!(!get(&rows, "c").redacted);
        assert_eq!(get(&rows, "d").value.as_deref(), Some("/not/a/url"));
    }
}
