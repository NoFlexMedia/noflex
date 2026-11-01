// SPDX-License-Identifier: AGPL-3.0-only

pub mod oidc;

#[cfg(target_arch = "wasm32")]
mod guest {
    wit_bindgen::generate!({
        path: "../../crates/noflex-plugin-wit/wit",
        world: "external-auth-plugin",
        generate_all,
    });

    #[unsafe(link_section = "noflex:manifest")]
    #[used]
    static NOFLEX_MANIFEST: [u8; include_bytes!("manifest.toml").len()] =
        *include_bytes!("manifest.toml");

    use exports::noflex::plugin::{background, external_auth};
    use noflex::plugin::{host_config, host_net, host_secrets};

    use crate::oidc::{self, Config, Discovery, TokenAuthMethod, TokenResponse, UserInfo};

    struct Oidc;

    fn plugin_error(message: impl Into<String>) -> external_auth::PluginError {
        external_auth::PluginError::BadInput(message.into())
    }

    fn unavailable(message: impl Into<String>) -> external_auth::PluginError {
        external_auth::PluginError::Unavailable(message.into())
    }

    fn config() -> Result<Config, external_auth::PluginError> {
        oidc::parse_config(&host_config::read()).map_err(plugin_error)
    }

    async fn request(
        method: &str,
        url: String,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, external_auth::PluginError> {
        let response = host_net::request(host_net::HttpRequest {
            method: method.into(),
            url,
            headers,
            body,
            timeout_ms: Some(15_000),
        })
        .await
        .map_err(|error| unavailable(format!("OIDC request failed: {error:?}")))?;
        if !(200..300).contains(&response.status) {
            return Err(unavailable(format!(
                "OIDC endpoint answered {}",
                response.status
            )));
        }
        Ok(response.body)
    }

    async fn discovery(config: &Config) -> Result<Discovery, external_auth::PluginError> {
        let body = request(
            "GET",
            oidc::discovery_url(&config.issuer).map_err(plugin_error)?,
            vec![("accept".into(), "application/json".into())],
            None,
        )
        .await?;
        oidc::parse_discovery(&config.issuer, &body).map_err(plugin_error)
    }

    impl external_auth::Guest for Oidc {
        async fn start(
            callback_uri: String,
            state: String,
            pkce_challenge: String,
        ) -> Result<String, external_auth::PluginError> {
            let config = config()?;
            let discovery = discovery(&config).await?;
            oidc::authorization_url(
                &discovery.authorization_endpoint,
                &config,
                &callback_uri,
                &state,
                &pkce_challenge,
            )
            .map_err(plugin_error)
        }

        async fn complete(
            callback_uri: String,
            code: String,
            pkce_verifier: String,
        ) -> Result<external_auth::VerifiedIdentity, external_auth::PluginError> {
            let config = config()?;
            let discovery = discovery(&config).await?;
            let secret = host_secrets::read("client_secret".into())
                .await
                .map_err(|error| unavailable(format!("client_secret unavailable: {error:?}")))?;
            if secret.is_empty() {
                return Err(unavailable("client_secret is not configured"));
            }
            let method = oidc::token_auth_method(&config, &discovery).map_err(unavailable)?;
            let mut headers = vec![
                ("accept".into(), "application/json".into()),
                (
                    "content-type".into(),
                    "application/x-www-form-urlencoded".into(),
                ),
            ];
            if method == TokenAuthMethod::ClientSecretBasic {
                headers.push((
                    "authorization".into(),
                    oidc::basic_authorization(&config.client_id, &secret),
                ));
            }
            let body = request(
                "POST",
                discovery.token_endpoint,
                headers,
                Some(oidc::token_body(
                    &config,
                    method,
                    &secret,
                    &callback_uri,
                    &code,
                    &pkce_verifier,
                )),
            )
            .await?;
            let token: TokenResponse = serde_json::from_slice(&body)
                .map_err(|error| unavailable(format!("invalid token response: {error}")))?;
            oidc::check_token_response(&token).map_err(unavailable)?;
            let id_token = token
                .id_token
                .as_deref()
                .ok_or_else(|| unavailable("token response omitted id_token"))?;
            let body = request(
                "GET",
                discovery.userinfo_endpoint,
                vec![
                    ("accept".into(), "application/json".into()),
                    (
                        "authorization".into(),
                        format!("Bearer {}", token.access_token),
                    ),
                ],
                None,
            )
            .await?;
            let user: UserInfo = serde_json::from_slice(&body)
                .map_err(|error| unavailable(format!("invalid UserInfo response: {error}")))?;
            if user.sub.is_empty() {
                return Err(unavailable("UserInfo sub is empty"));
            }
            oidc::check_id_token(id_token, &config, &user.sub).map_err(unavailable)?;
            Ok(external_auth::VerifiedIdentity {
                issuer: config.issuer,
                subject: user.sub,
                preferred_username: user.preferred_username,
                display_name: user.name,
                email: user.email,
            })
        }
    }

    impl background::Guest for Oidc {
        async fn run_task(
            _task: String,
            _cursor: Option<String>,
            _payload: Option<String>,
        ) -> Result<background::TaskOutcome, background::PluginError> {
            Ok(background::TaskOutcome::Done)
        }
    }

    export!(Oidc);
}
