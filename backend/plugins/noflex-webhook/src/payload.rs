// SPDX-License-Identifier: AGPL-3.0-only
//
// Pure mapping core of the webhook plugin: notify-event → JSON body,
// config-stanza parsing, request header construction. Nothing in here
// touches the WIT bindings (the `From<types::NotifyEvent>` conversion
// lives in the wasm-only guest module), so `cargo test` exercises this
// file on the host target.

use serde::Deserialize;
use serde_json::json;

/// Parsed `[plugins.per_plugin_config.webhook]` stanza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub url: String,
    pub auth_secret_key: Option<String>,
}

/// Parse the TOML blob `host-config.read` hands over.
///
/// The server renders the stanza with `toml::Value::to_string()`, which
/// produces a bare inline table (`{ url = "..." }`) rather than a TOML
/// document, so the plain parse is retried with the blob wrapped as a
/// value. Errors are strings because the guest's only error channel for
/// operator misconfig is the log.
pub fn parse_config(raw: &str) -> Result<Config, String> {
    #[derive(Deserialize)]
    struct Stanza {
        url: Option<String>,
        auth_secret_key: Option<String>,
    }
    #[derive(Deserialize)]
    struct Wrapped {
        stanza: Stanza,
    }

    let trimmed = raw.trim();
    let parsed: Stanza = if trimmed.starts_with('{') {
        toml::from_str::<Wrapped>(&format!("stanza = {trimmed}"))
            .map(|w| w.stanza)
            .map_err(|e| format!("invalid TOML: {e}"))
    } else {
        toml::from_str::<Stanza>(trimmed).map_err(|e| format!("invalid TOML: {e}"))
    }?;
    let url = parsed
        .url
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| "missing or empty `url`".to_string())?;
    Ok(Config {
        url,
        auth_secret_key: parsed.auth_secret_key,
    })
}

/// One outbound notification. Mirrors the WIT `notify-event` variant
/// set with owned plain types so the JSON mapping is testable off-wasm;
/// field names match the WIT records verbatim.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    WorkAdded {
        work: String,
        title: String,
    },
    WorkPlayed {
        user: String,
        work: String,
        title: String,
    },
    ScanStarted {
        library_name: String,
    },
    ScanCompleted {
        library_name: String,
        added: u64,
        updated: u64,
        removed: u64,
    },
    Favorite {
        user: String,
        work: String,
        title: String,
        favorite: bool,
    },
    ProgressAtStop {
        user: String,
        work: String,
        title: String,
        position_ms: u64,
    },
    PlaybackStarted {
        user: String,
        work: String,
        title: String,
        media_file: String,
    },
    PlaybackStopped {
        user: String,
        work: String,
        title: String,
        media_file: String,
    },
    Custom {
        event: String,
        payload_json: String,
    },
}

impl Event {
    /// The `event` value in the POST body. Matches `DomainEvent::kind_str`,
    /// which is also what a manifest `events` filter subscribes to.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Event::WorkAdded { .. } => "work-added",
            Event::WorkPlayed { .. } => "work-played",
            Event::ScanStarted { .. } => "scan-started",
            Event::ScanCompleted { .. } => "scan-completed",
            Event::Favorite { .. } => "favorite",
            Event::ProgressAtStop { .. } => "progress-at-stop",
            Event::PlaybackStarted { .. } => "playback-started",
            Event::PlaybackStopped { .. } => "playback-stopped",
            Event::Custom { .. } => "custom",
        }
    }

    /// The JSON body POSTed to the configured URL:
    /// `{"event": "<kind>", "data": { ...variant fields... }}`.
    /// A custom event's `payload_json` stays a string containing JSON —
    /// receivers decode it if they care; re-embedding it would make the
    /// shape depend on the sender's payload.
    pub fn body(&self) -> String {
        let data = match self {
            Event::WorkAdded { work, title } => json!({ "work": work, "title": title }),
            Event::WorkPlayed { user, work, title } => {
                json!({ "user": user, "work": work, "title": title })
            }
            Event::ScanStarted { library_name } => json!({ "library_name": library_name }),
            Event::ScanCompleted {
                library_name,
                added,
                updated,
                removed,
            } => json!({
                "library_name": library_name,
                "added": added,
                "updated": updated,
                "removed": removed,
            }),
            Event::Favorite {
                user,
                work,
                title,
                favorite,
            } => json!({
                "user": user,
                "work": work,
                "title": title,
                "favorite": favorite,
            }),
            Event::ProgressAtStop {
                user,
                work,
                title,
                position_ms,
            } => json!({
                "user": user,
                "work": work,
                "title": title,
                "position_ms": position_ms,
            }),
            Event::PlaybackStarted {
                user,
                work,
                title,
                media_file,
            }
            | Event::PlaybackStopped {
                user,
                work,
                title,
                media_file,
            } => json!({
                "user": user,
                "work": work,
                "title": title,
                "media_file": media_file,
            }),
            Event::Custom {
                event,
                payload_json,
            } => json!({ "event": event, "payload_json": payload_json }),
        };
        json!({ "event": self.kind_str(), "data": data }).to_string()
    }
}

/// Request headers for the POST. `auth` is the resolved secret value —
/// its absence (no key configured, or the read failed) just drops the
/// header rather than failing the notification.
pub fn headers(auth: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
    if let Some(token) = auth {
        headers.push(("authorization".to_string(), format!("Bearer {token}")));
    }
    headers
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn body_json(event: &Event) -> Value {
        serde_json::from_str(&event.body()).expect("body is valid JSON")
    }

    #[test]
    fn config_parses_plain_document() {
        let cfg = parse_config(
            "url = \"https://ntfy.example.com/noflex\"\nauth_secret_key = \"webhook.auth\"\n",
        )
        .expect("valid stanza");
        assert_eq!(cfg.url, "https://ntfy.example.com/noflex");
        assert_eq!(cfg.auth_secret_key.as_deref(), Some("webhook.auth"));
    }

    #[test]
    fn config_parses_server_rendered_inline_table() {
        // `toml::Value::to_string()` output is what actually arrives at
        // runtime: a bare inline table, not a document.
        let cfg = parse_config("{ url = \"http://127.0.0.1:8901/hook\" }").expect("inline table");
        assert_eq!(cfg.url, "http://127.0.0.1:8901/hook");
        assert_eq!(cfg.auth_secret_key, None);
    }

    #[test]
    fn config_rejects_missing_or_empty_url() {
        for raw in [
            "auth_secret_key = \"webhook.auth\"",
            "url = \"\"",
            "{ auth_secret_key = \"webhook.auth\" }",
            "",
        ] {
            let err = parse_config(raw).expect_err("no url, no config");
            assert!(err.contains("url"), "error names the missing key: {err}");
        }
    }

    #[test]
    fn config_rejects_malformed_toml() {
        let err = parse_config("url = [unterminated").expect_err("garbage is rejected");
        assert!(err.contains("invalid TOML"), "{err}");
    }

    #[test]
    fn work_added_body() {
        let v = body_json(&Event::WorkAdded {
            work: "01956c1f-b8a3-77a1-8000-000000000001".into(),
            title: "The Matrix".into(),
        });
        assert_eq!(v["event"], "work-added");
        assert_eq!(v["data"]["work"], "01956c1f-b8a3-77a1-8000-000000000001");
        assert_eq!(v["data"]["title"], "The Matrix");
        assert_eq!(v["data"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn work_played_body() {
        let v = body_json(&Event::WorkPlayed {
            user: "u-1".into(),
            work: "w-1".into(),
            title: "Episode 4".into(),
        });
        assert_eq!(v["event"], "work-played");
        assert_eq!(
            v["data"],
            json!({"user": "u-1", "work": "w-1", "title": "Episode 4"})
        );
    }

    #[test]
    fn scan_started_body() {
        let v = body_json(&Event::ScanStarted {
            library_name: "Movies".into(),
        });
        assert_eq!(v["event"], "scan-started");
        assert_eq!(v["data"], json!({"library_name": "Movies"}));
    }

    #[test]
    fn scan_completed_body() {
        let v = body_json(&Event::ScanCompleted {
            library_name: "Movies".into(),
            added: 3,
            updated: 1,
            removed: 0,
        });
        assert_eq!(v["event"], "scan-completed");
        assert_eq!(
            v["data"],
            json!({"library_name": "Movies", "added": 3, "updated": 1, "removed": 0})
        );
    }

    #[test]
    fn favorite_body() {
        let v = body_json(&Event::Favorite {
            user: "u-1".into(),
            work: "w-1".into(),
            title: "Alien".into(),
            favorite: true,
        });
        assert_eq!(v["event"], "favorite");
        assert_eq!(
            v["data"],
            json!({"user": "u-1", "work": "w-1", "title": "Alien", "favorite": true})
        );
    }

    #[test]
    fn progress_at_stop_body() {
        let v = body_json(&Event::ProgressAtStop {
            user: "u-1".into(),
            work: "w-1".into(),
            title: "Alien".into(),
            position_ms: 540_000,
        });
        assert_eq!(v["event"], "progress-at-stop");
        assert_eq!(v["data"]["position_ms"], 540_000);
        assert_eq!(v["data"]["user"], "u-1");
    }

    #[test]
    fn playback_bodies_carry_the_media_file() {
        for (event, kind) in [
            (
                Event::PlaybackStarted {
                    user: "u-1".into(),
                    work: "w-1".into(),
                    title: "Alien".into(),
                    media_file: "mf-1".into(),
                },
                "playback-started",
            ),
            (
                Event::PlaybackStopped {
                    user: "u-1".into(),
                    work: "w-1".into(),
                    title: "Alien".into(),
                    media_file: "mf-1".into(),
                },
                "playback-stopped",
            ),
        ] {
            let v = body_json(&event);
            assert_eq!(v["event"], kind);
            assert_eq!(v["data"]["media_file"], "mf-1");
            assert_eq!(v["data"].as_object().unwrap().len(), 4);
        }
    }

    #[test]
    fn custom_body_keeps_payload_as_a_string() {
        let v = body_json(&Event::Custom {
            event: "my-plugin.thing-happened".into(),
            payload_json: "{\"count\":2}".into(),
        });
        assert_eq!(v["event"], "custom");
        assert_eq!(v["data"]["event"], "my-plugin.thing-happened");
        // Not double-decoded: receivers get the JSON as a string.
        assert_eq!(v["data"]["payload_json"], "{\"count\":2}");
    }

    #[test]
    fn headers_without_auth() {
        assert_eq!(
            headers(None),
            vec![("content-type".to_string(), "application/json".to_string())]
        );
    }

    #[test]
    fn headers_with_auth_adds_bearer() {
        let h = headers(Some("s3cret"));
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].0, "content-type");
        assert_eq!(
            h[1],
            ("authorization".to_string(), "Bearer s3cret".to_string())
        );
    }
}
