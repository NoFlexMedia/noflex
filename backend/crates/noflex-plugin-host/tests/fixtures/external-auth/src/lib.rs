// SPDX-License-Identifier: AGPL-3.0-only

#![cfg(target_arch = "wasm32")]

wit_bindgen::generate!({
    path: "../../../../noflex-plugin-wit/wit",
    world: "external-auth-plugin",
    generate_all,
});

#[unsafe(link_section = "noflex:manifest")]
#[used]
static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
    *include_bytes!("manifest.toml");

use exports::noflex::plugin::{background, external_auth};
use noflex::plugin::types::PluginError;

struct Fixture;

impl external_auth::Guest for Fixture {
    async fn start(
        callback_uri: String,
        state: String,
        pkce_challenge: String,
    ) -> Result<String, PluginError> {
        Ok(format!(
            "https://id.example/authorize?redirect_uri={callback_uri}&state={state}&challenge={pkce_challenge}"
        ))
    }

    async fn complete(
        callback_uri: String,
        code: String,
        pkce_verifier: String,
    ) -> Result<external_auth::VerifiedIdentity, PluginError> {
        if code != "fixture-code" || pkce_verifier.is_empty() {
            return Err(PluginError::BadInput("invalid fixture callback".into()));
        }
        Ok(external_auth::VerifiedIdentity {
            issuer: "https://id.example".into(),
            subject: format!("subject:{callback_uri}"),
            preferred_username: Some("fixture-user".into()),
            display_name: Some("Fixture User".into()),
            email: Some("fixture@example.test".into()),
        })
    }
}

impl background::Guest for Fixture {
    async fn run_task(
        _task: String,
        _cursor: Option<String>,
        _payload: Option<String>,
    ) -> Result<background::TaskOutcome, PluginError> {
        Ok(background::TaskOutcome::Done)
    }
}

export!(Fixture);
