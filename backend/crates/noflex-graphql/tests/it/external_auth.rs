// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::{Arc, Mutex};

use async_graphql::Request;
use noflex_auth::{JwtConfig, Principal, verify_jwt};
use noflex_core::{SessionId, UserId};
use noflex_graphql::{
    ExternalAuthChallenge, ExternalAuthError, ExternalAuthService, SchemaContext, build_schema,
};
use noflex_plugin_host::{EngineConfig, PluginRegistry, build_engine};
use noflex_storage::{Storage, domain::ExternalIdentity};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;

const JWT_SECRET: &[u8] = b"external-auth-test-secret-32bytes";

const NONCE: &str = "browser-nonce-0123456789abcdefghijklmnopqrstuvwxyz";

#[derive(Default)]
struct FakeExternalAuth {
    starts: Mutex<Vec<(String, Option<UserId>, String)>>,
    /// `(code, nonce, user)` for the one outstanding login exchange.
    exchange: Mutex<Option<(String, String, UserId)>>,
    links: Mutex<Vec<(String, String, UserId)>>,
}

impl FakeExternalAuth {
    fn record(
        &self,
        provider: &str,
        user: Option<UserId>,
        nonce: &str,
    ) -> Result<ExternalAuthChallenge, ExternalAuthError> {
        self.starts.lock().expect("starts lock").push((
            provider.to_owned(),
            user,
            nonce.to_owned(),
        ));
        Ok(ExternalAuthChallenge {
            authorization_url: format!("https://id.example/authorize?provider={provider}"),
        })
    }
}

#[async_trait::async_trait]
impl ExternalAuthService for FakeExternalAuth {
    fn available(&self) -> bool {
        true
    }

    async fn start_login(
        &self,
        provider: &str,
        client_nonce: &str,
    ) -> Result<ExternalAuthChallenge, ExternalAuthError> {
        self.record(provider, None, client_nonce)
    }

    async fn start_link(
        &self,
        provider: &str,
        user: UserId,
        client_nonce: &str,
    ) -> Result<ExternalAuthChallenge, ExternalAuthError> {
        self.record(provider, Some(user), client_nonce)
    }

    async fn complete_login(
        &self,
        code: &str,
        client_nonce: &str,
    ) -> Result<UserId, ExternalAuthError> {
        match self.exchange.lock().expect("exchange lock").take() {
            Some((expected, nonce, user)) if expected == code && nonce == client_nonce => Ok(user),
            _ => Err(ExternalAuthError::InvalidCode),
        }
    }

    async fn complete_link(
        &self,
        code: &str,
        client_nonce: &str,
        user: UserId,
    ) -> Result<ExternalIdentity, ExternalAuthError> {
        self.links.lock().expect("links lock").push((
            code.to_owned(),
            client_nonce.to_owned(),
            user,
        ));
        Ok(ExternalIdentity {
            plugin_id: "oidc".into(),
            issuer: "https://id.example".into(),
            subject: "linked".into(),
            user_id: user,
            username: None,
            display_name: None,
            email: None,
            linked_at: OffsetDateTime::now_utc(),
        })
    }
}

async fn fixture() -> (
    noflex_graphql::NoflexSchema,
    Arc<TursoStore>,
    Arc<FakeExternalAuth>,
    Arc<JwtConfig>,
) {
    let store = Arc::new(TursoStore::open_temp().await.expect("temp store"));
    store.migrate().await.expect("migrate");
    let service = Arc::new(FakeExternalAuth::default());
    let jwt = Arc::new(JwtConfig::dev(JWT_SECRET).expect("JWT config"));
    let engine = build_engine(&EngineConfig::default()).expect("engine");
    let registry = Arc::new(PluginRegistry::new(engine).expect("registry"));
    let schema = build_schema(SchemaContext {
        storage: Some(store.clone()),
        plugins: Some(registry),
        jwt: Some(jwt.clone()),
        external_auth: Some(service.clone()),
        ..Default::default()
    });
    (schema, store, service, jwt)
}

async fn setup_admin(schema: &noflex_graphql::NoflexSchema, jwt: &JwtConfig) -> Principal {
    let response = schema
        .execute(
            r#"mutation { completeSetupWizard(username: "admin", password: "hunter2") { accessToken } }"#,
        )
        .await;
    assert!(response.errors.is_empty(), "setup: {:?}", response.errors);
    let token =
        response.data.into_json().expect("setup JSON")["completeSetupWizard"]["accessToken"]
            .as_str()
            .expect("access token")
            .to_owned();
    let claims = verify_jwt(jwt, &token).expect("verify token");
    Principal {
        user_id: UserId::from_uuid(claims.sub.parse().expect("user UUID")),
        is_admin: claims.is_admin,
        session_id: Some(SessionId::from_uuid(
            claims.sid.parse().expect("session UUID"),
        )),
    }
}

#[tokio::test]
async fn provider_and_login_surfaces_are_public_and_exchange_is_one_use() {
    let (schema, _store, service, jwt) = fixture().await;
    let admin = setup_admin(&schema, &jwt).await;

    let response = schema
        .execute("{ externalAuthProviders { id displayName } }")
        .await;
    assert!(
        response.errors.is_empty(),
        "providers: {:?}",
        response.errors
    );

    let response = schema
        .execute(format!(
            r#"mutation {{ startExternalLogin(provider: "oidc", clientNonce: "{NONCE}") {{ authorizationUrl }} }}"#
        ))
        .await;
    assert!(response.errors.is_empty(), "start: {:?}", response.errors);
    assert_eq!(
        service.starts.lock().expect("starts lock").as_slice(),
        &[("oidc".into(), None, NONCE.into())]
    );

    *service.exchange.lock().expect("exchange lock") =
        Some(("one-use".into(), NONCE.into(), admin.user_id));
    let complete = format!(
        r#"mutation {{ completeExternalLogin(code: "one-use", clientNonce: "{NONCE}") {{ token refreshToken user {{ id }} }} }}"#
    );
    let response = schema.execute(complete.clone()).await;
    assert!(
        response.errors.is_empty(),
        "complete: {:?}",
        response.errors
    );
    let response = schema.execute(complete).await;
    assert!(
        response
            .errors
            .iter()
            .any(|error| error.message == "external login code is invalid or expired"),
        "replay: {:?}",
        response.errors
    );
}

#[tokio::test]
async fn identity_queries_link_and_revoke_require_authentication() {
    let (schema, store, service, jwt) = fixture().await;
    let admin = setup_admin(&schema, &jwt).await;

    let start_link = format!(
        r#"mutation {{ startExternalIdentityLink(provider: "oidc", clientNonce: "{NONCE}") {{ authorizationUrl }} }}"#
    );
    let complete_link = format!(
        r#"mutation {{ completeExternalIdentityLink(code: "link-code", clientNonce: "{NONCE}") {{ pluginId userId }} }}"#
    );
    for query in [
        "{ myExternalIdentities { pluginId } }".to_owned(),
        start_link.clone(),
        complete_link.clone(),
        r#"mutation { revokeExternalIdentity(provider: "oidc") { pluginId } }"#.to_owned(),
    ] {
        let response = schema.execute(query.as_str()).await;
        assert!(
            response
                .errors
                .iter()
                .any(|error| error.message == "authentication required"),
            "anonymous query {query}: {:?}",
            response.errors
        );
    }

    assert!(
        service.links.lock().expect("links lock").is_empty(),
        "anonymous completion never reaches the service"
    );
    let response = schema.execute(Request::new(start_link).data(admin)).await;
    assert!(
        response.errors.is_empty(),
        "link start: {:?}",
        response.errors
    );
    assert_eq!(
        service.starts.lock().expect("starts lock").last(),
        Some(&("oidc".into(), Some(admin.user_id), NONCE.into()))
    );
    let response = schema
        .execute(Request::new(complete_link).data(admin))
        .await;
    assert!(
        response.errors.is_empty(),
        "link complete: {:?}",
        response.errors
    );
    assert_eq!(
        service.links.lock().expect("links lock").as_slice(),
        &[("link-code".into(), NONCE.into(), admin.user_id)],
        "the caller's own id is what the service checks the code against"
    );

    store
        .link_external_identity(&ExternalIdentity {
            plugin_id: "oidc".into(),
            issuer: "https://id.example".into(),
            subject: "admin".into(),
            user_id: admin.user_id,
            username: Some("admin".into()),
            display_name: None,
            email: None,
            linked_at: OffsetDateTime::now_utc(),
        })
        .await
        .expect("seed identity");
    let response = schema
        .execute(
            Request::new(r#"mutation { revokeExternalIdentity(provider: "oidc") { pluginId } }"#)
                .data(admin),
        )
        .await;
    assert!(response.errors.is_empty(), "revoke: {:?}", response.errors);
    assert_eq!(
        response.data.into_json().expect("revoke JSON")["revokeExternalIdentity"]["pluginId"],
        "oidc"
    );
}
