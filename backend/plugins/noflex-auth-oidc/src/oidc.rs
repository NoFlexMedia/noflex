// SPDX-License-Identifier: AGPL-3.0-only

use serde::Deserialize;
use url::{Host, Url};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub issuer: String,
    pub client_id: String,
    /// Space-separated; must include `openid`.
    #[serde(default = "default_scopes")]
    pub scopes: String,
    /// How the client authenticates at the token endpoint. Unset picks
    /// from the discovery document's `token_endpoint_auth_methods_supported`
    /// (preferring `client_secret_basic`, the OIDC default).
    #[serde(default)]
    pub token_endpoint_auth_method: Option<TokenAuthMethod>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenAuthMethod {
    ClientSecretBasic,
    ClientSecretPost,
}

impl TokenAuthMethod {
    fn as_str(self) -> &'static str {
        match self {
            TokenAuthMethod::ClientSecretBasic => "client_secret_basic",
            TokenAuthMethod::ClientSecretPost => "client_secret_post",
        }
    }
}

fn default_scopes() -> String {
    "openid profile email".into()
}

#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
    /// Absent means the spec default, `client_secret_basic` only.
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    #[serde(default)]
    pub id_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserInfo {
    pub sub: String,
    pub preferred_username: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
}

pub fn parse_config(raw: &str) -> Result<Config, String> {
    let config: Config = toml::from_str(raw).map_err(|e| format!("invalid OIDC config: {e}"))?;
    validate_issuer(&config.issuer)?;
    if config.client_id.trim().is_empty() {
        return Err("OIDC client_id is empty".into());
    }
    if !config
        .scopes
        .split_ascii_whitespace()
        .any(|s| s == "openid")
    {
        return Err("OIDC scopes must include `openid`".into());
    }
    Ok(config)
}

/// The token-endpoint client authentication to use: the configured method,
/// else `client_secret_basic` when the IdP supports it (or lists nothing,
/// the spec default), else `client_secret_post`.
pub fn token_auth_method(
    config: &Config,
    discovery: &Discovery,
) -> Result<TokenAuthMethod, String> {
    let supported = &discovery.token_endpoint_auth_methods_supported;
    let supports = |m: TokenAuthMethod| {
        (supported.is_empty() && m == TokenAuthMethod::ClientSecretBasic)
            || supported.iter().any(|s| s == m.as_str())
    };
    if let Some(method) = config.token_endpoint_auth_method {
        return Ok(method);
    }
    [
        TokenAuthMethod::ClientSecretBasic,
        TokenAuthMethod::ClientSecretPost,
    ]
    .into_iter()
    .find(|m| supports(*m))
    .ok_or_else(|| {
        "OIDC provider supports neither client_secret_basic nor client_secret_post".into()
    })
}

/// Validate the token response before its access token is used: the token
/// type must be Bearer (RFC 6750, case-insensitive) and the access token
/// non-empty.
pub fn check_token_response(token: &TokenResponse) -> Result<(), String> {
    if !token.token_type.eq_ignore_ascii_case("bearer") {
        return Err(format!(
            "OIDC token_type `{}` is not Bearer",
            token.token_type
        ));
    }
    if token.access_token.is_empty() {
        return Err("token response omitted access_token".into());
    }
    Ok(())
}

pub fn discovery_url(issuer: &str) -> Result<String, String> {
    validate_issuer(issuer)?;
    Ok(format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    ))
}

pub fn parse_discovery(configured_issuer: &str, body: &[u8]) -> Result<Discovery, String> {
    validate_issuer(configured_issuer)?;
    let discovery: Discovery =
        serde_json::from_slice(body).map_err(|e| format!("invalid discovery document: {e}"))?;
    if discovery.issuer != configured_issuer {
        return Err("discovery issuer does not match configured issuer".into());
    }
    validate_issuer(&discovery.issuer)?;
    validate_endpoint("authorization_endpoint", &discovery.authorization_endpoint)?;
    validate_endpoint("token_endpoint", &discovery.token_endpoint)?;
    validate_endpoint("userinfo_endpoint", &discovery.userinfo_endpoint)?;
    Ok(discovery)
}

pub fn authorization_url(
    endpoint: &str,
    config: &Config,
    callback_uri: &str,
    state: &str,
    challenge: &str,
) -> Result<String, String> {
    let mut url = validate_endpoint("authorization_endpoint", endpoint)?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", callback_uri)
        .append_pair("scope", &config.scopes)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url.into())
}

/// Form body for the authorization-code exchange. With
/// `client_secret_post` the credentials ride in the body; with
/// `client_secret_basic` they go in [`basic_authorization`] instead.
pub fn token_body(
    config: &Config,
    method: TokenAuthMethod,
    secret: &str,
    callback_uri: &str,
    code: &str,
    verifier: &str,
) -> Vec<u8> {
    let mut body = format!(
        "grant_type=authorization_code&redirect_uri={}&code={}&code_verifier={}",
        percent_encode(callback_uri),
        percent_encode(code),
        percent_encode(verifier),
    );
    if method == TokenAuthMethod::ClientSecretPost {
        body.push_str(&format!(
            "&client_id={}&client_secret={}",
            percent_encode(&config.client_id),
            percent_encode(secret),
        ));
    }
    body.into_bytes()
}

/// `Authorization` header value for `client_secret_basic`: RFC 6749 §2.3.1
/// form-encodes both halves before base64.
pub fn basic_authorization(client_id: &str, secret: &str) -> String {
    let credentials = format!("{}:{}", percent_encode(client_id), percent_encode(secret));
    format!("Basic {}", base64_encode(credentials.as_bytes()))
}

/// Check the ID token's `iss`, `aud`, and `sub` against the configured
/// client and the UserInfo subject (OIDC Core 3.1.3.7, 5.3.2). The JWS
/// signature is not verified: the token came directly from the token
/// endpoint over the TLS connection the host vetted, which OIDC Core
/// 3.1.3.7 accepts in place of signature validation for the code flow.
pub fn check_id_token(id_token: &str, config: &Config, userinfo_sub: &str) -> Result<(), String> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Audience {
        One(String),
        Many(Vec<String>),
    }
    #[derive(Deserialize)]
    struct Claims {
        iss: String,
        aud: Audience,
        sub: String,
        #[serde(default)]
        azp: Option<String>,
    }

    let payload = id_token
        .split('.')
        .nth(1)
        .ok_or("OIDC id_token is not a JWT")?;
    let payload = base64url_decode(payload).ok_or("OIDC id_token payload is not base64url")?;
    let claims: Claims =
        serde_json::from_slice(&payload).map_err(|e| format!("invalid id_token claims: {e}"))?;
    if claims.iss != config.issuer {
        return Err("id_token issuer does not match configured issuer".into());
    }
    let audiences = match claims.aud {
        Audience::One(aud) => vec![aud],
        Audience::Many(auds) => auds,
    };
    if !audiences.iter().any(|aud| aud == &config.client_id) {
        return Err("id_token audience does not include this client".into());
    }
    if audiences.len() > 1 && claims.azp.as_deref() != Some(config.client_id.as_str()) {
        return Err("multi-audience id_token is not authorized for this client".into());
    }
    if claims.sub != userinfo_sub {
        return Err("UserInfo sub does not match id_token sub".into());
    }
    Ok(())
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64_ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Unpadded (or padded) base64url, as JWT segments use.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut nbits = 0;
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    for byte in input.trim_end_matches('=').bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
            bits &= (1 << nbits) - 1;
        }
    }
    Some(out)
}

pub fn percent_encode(input: &str) -> String {
    let mut output = String::new();
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            output.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}

fn validate_issuer(issuer: &str) -> Result<Url, String> {
    let url = validate_https_url("issuer", issuer)?;
    if url.query().is_some() {
        return Err("OIDC issuer must not contain a query".into());
    }
    Ok(url)
}

fn validate_endpoint(name: &str, endpoint: &str) -> Result<Url, String> {
    validate_https_url(name, endpoint)
}

fn validate_https_url(name: &str, value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|error| format!("invalid OIDC {name}: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(format!("OIDC {name} must be an absolute HTTP(S) URL"));
    }
    if has_userinfo(value) {
        return Err(format!("OIDC {name} must not contain userinfo"));
    }
    if url.fragment().is_some() {
        return Err(format!("OIDC {name} must not contain a fragment"));
    }
    if url.scheme() != "https" && !is_loopback(&url) {
        return Err(format!("OIDC {name} must use HTTPS except on loopback"));
    }
    Ok(url)
}

fn has_userinfo(value: &str) -> bool {
    value
        .split_once("://")
        .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
        .is_some_and(|authority| authority.contains('@'))
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(host)) => host.is_loopback(),
        Some(Host::Ipv6(host)) => host.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discovery(issuer: &str, authorization: &str, token: &str, userinfo: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "issuer": issuer,
            "authorization_endpoint": authorization,
            "token_endpoint": token,
            "userinfo_endpoint": userinfo,
        }))
        .expect("discovery JSON")
    }

    #[test]
    fn discovery_requires_exact_issuer_and_safe_endpoints() {
        let body = discovery(
            "https://id.example",
            "https://id.example/auth?tenant=one",
            "https://id.example/token?tenant=one",
            "https://id.example/userinfo?schema=openid",
        );
        assert!(parse_discovery("https://id.example", &body).is_ok());
        assert!(parse_discovery("https://id.example/", &body).is_err());

        for endpoint in [
            "http://id.example/auth",
            "https://user@id.example/auth",
            "https://id.example/auth#fragment",
            "javascript:alert(1)",
        ] {
            let body = discovery(
                "https://id.example",
                endpoint,
                "https://id.example/token",
                "https://id.example/userinfo",
            );
            assert!(parse_discovery("https://id.example", &body).is_err());
        }
    }

    #[test]
    fn authorization_url_safely_extends_existing_query() {
        let config = Config {
            issuer: "https://id.example".into(),
            client_id: "client id&admin=true".into(),
            scopes: default_scopes(),
            token_endpoint_auth_method: None,
        };
        let value = authorization_url(
            "https://id.example/auth?tenant=one",
            &config,
            "http://localhost/auth/oidc/callback?x=1&y=2",
            "state&prompt=none",
            "challenge",
        )
        .expect("authorization URL");
        let url = Url::parse(&value).expect("URL parses");
        let pairs = url
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(pairs.get("tenant").map(|v| v.as_ref()), Some("one"));
        assert_eq!(
            pairs.get("client_id").map(|v| v.as_ref()),
            Some("client id&admin=true")
        );
        assert_eq!(
            pairs.get("state").map(|v| v.as_ref()),
            Some("state&prompt=none")
        );
        assert_eq!(
            pairs.get("code_challenge_method").map(|v| v.as_ref()),
            Some("S256")
        );
    }

    #[test]
    fn issuer_parser_allows_only_secure_or_loopback_urls() {
        for issuer in [
            "https://id.example/tenant",
            "http://localhost:9000",
            "http://127.0.0.1:9000",
            "http://[::1]:9000",
        ] {
            assert!(
                parse_config(&format!("issuer='{issuer}'\nclient_id='x'")).is_ok(),
                "rejected {issuer}"
            );
        }
        for issuer in [
            "http://id.example",
            "https://user@id.example",
            "https://@id.example",
            "https://id.example?tenant=one",
            "https://id.example#fragment",
            "id.example",
        ] {
            assert!(
                parse_config(&format!("issuer='{issuer}'\nclient_id='x'")).is_err(),
                "accepted {issuer}"
            );
        }
    }

    fn config() -> Config {
        parse_config("issuer='https://id.example'\nclient_id='noflex'").expect("config")
    }

    fn discovery_with(methods: &[&str]) -> Discovery {
        Discovery {
            issuer: "https://id.example".into(),
            authorization_endpoint: "https://id.example/auth".into(),
            token_endpoint: "https://id.example/token".into(),
            userinfo_endpoint: "https://id.example/userinfo".into(),
            token_endpoint_auth_methods_supported: methods.iter().map(|m| m.to_string()).collect(),
        }
    }

    fn jwt(claims: serde_json::Value) -> String {
        let encode = |bytes: &[u8]| {
            base64_encode(bytes)
                .trim_end_matches('=')
                .replace('+', "-")
                .replace('/', "_")
        };
        format!(
            "{}.{}.sig",
            encode(br#"{"alg":"RS256"}"#),
            encode(&serde_json::to_vec(&claims).unwrap())
        )
    }

    #[test]
    fn scopes_must_include_openid() {
        assert!(
            parse_config("issuer='https://id.example'\nclient_id='x'\nscopes='profile email'")
                .is_err()
        );
        assert!(
            parse_config("issuer='https://id.example'\nclient_id='x'\nscopes='openidx'").is_err()
        );
        assert!(
            parse_config("issuer='https://id.example'\nclient_id='x'\nscopes='email openid'")
                .is_ok()
        );
    }

    #[test]
    fn token_auth_method_follows_config_then_discovery() {
        let basic = TokenAuthMethod::ClientSecretBasic;
        let post = TokenAuthMethod::ClientSecretPost;
        assert_eq!(
            token_auth_method(&config(), &discovery_with(&[])).unwrap(),
            basic
        );
        assert_eq!(
            token_auth_method(&config(), &discovery_with(&["client_secret_post"])).unwrap(),
            post
        );
        assert_eq!(
            token_auth_method(
                &config(),
                &discovery_with(&["client_secret_post", "client_secret_basic"])
            )
            .unwrap(),
            basic
        );
        assert!(token_auth_method(&config(), &discovery_with(&["private_key_jwt"])).is_err());
        let forced = parse_config(
            "issuer='https://id.example'\nclient_id='x'\ntoken_endpoint_auth_method='client_secret_post'",
        )
        .unwrap();
        assert_eq!(
            token_auth_method(&forced, &discovery_with(&[])).unwrap(),
            post
        );
    }

    #[test]
    fn credentials_go_in_exactly_one_place() {
        let config = config();
        let post = String::from_utf8(token_body(
            &config,
            TokenAuthMethod::ClientSecretPost,
            "s&cret",
            "https://cb",
            "code",
            "v",
        ))
        .unwrap();
        assert!(post.contains("client_secret=s%26cret"));
        let basic = String::from_utf8(token_body(
            &config,
            TokenAuthMethod::ClientSecretBasic,
            "s&cret",
            "https://cb",
            "code",
            "v",
        ))
        .unwrap();
        assert!(!basic.contains("client_secret") && !basic.contains("client_id"));
        // base64("noflex:s%26cret")
        assert_eq!(
            basic_authorization("noflex", "s&cret"),
            "Basic bm9mbGV4OnMlMjZjcmV0"
        );
    }

    #[test]
    fn token_type_must_be_bearer() {
        let token = |token_type: &str| TokenResponse {
            access_token: "at".into(),
            token_type: token_type.into(),
            id_token: None,
        };
        assert!(check_token_response(&token("Bearer")).is_ok());
        assert!(check_token_response(&token("bearer")).is_ok());
        assert!(check_token_response(&token("DPoP")).is_err());
        assert!(serde_json::from_str::<TokenResponse>(r#"{"access_token":"at"}"#).is_err());
    }

    #[test]
    fn id_token_claims_bind_issuer_audience_and_subject() {
        let config = config();
        let good = jwt(serde_json::json!({
            "iss": "https://id.example", "aud": "noflex", "sub": "u1"
        }));
        assert!(check_id_token(&good, &config, "u1").is_ok());
        assert!(
            check_id_token(&good, &config, "u2").is_err(),
            "sub mismatch"
        );
        let wrong_iss = jwt(serde_json::json!({
            "iss": "https://evil.example", "aud": "noflex", "sub": "u1"
        }));
        assert!(check_id_token(&wrong_iss, &config, "u1").is_err());
        let wrong_aud = jwt(serde_json::json!({
            "iss": "https://id.example", "aud": ["other"], "sub": "u1"
        }));
        assert!(check_id_token(&wrong_aud, &config, "u1").is_err());
        let multi = jwt(serde_json::json!({
            "iss": "https://id.example", "aud": ["noflex", "other"], "sub": "u1"
        }));
        assert!(check_id_token(&multi, &config, "u1").is_err(), "needs azp");
        let multi_azp = jwt(serde_json::json!({
            "iss": "https://id.example", "aud": ["noflex", "other"], "sub": "u1", "azp": "noflex"
        }));
        assert!(check_id_token(&multi_azp, &config, "u1").is_ok());
        assert!(check_id_token("not-a-jwt", &config, "u1").is_err());
    }
}
