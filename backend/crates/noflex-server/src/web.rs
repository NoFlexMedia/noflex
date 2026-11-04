// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use axum::{
    Extension, Router,
    body::Body,
    http::{HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use noflex_storage::Storage;
use tower_http::cors::{AllowOrigin, CorsLayer};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));
}

const NO_CACHE: &str = "no-cache";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

pub fn routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().fallback(get(serve))
}

async fn serve(storage: Option<Extension<Arc<dyn Storage>>>, uri: Uri) -> Response {
    let linked = match &storage {
        Some(Extension(storage)) => match storage.linked_server_origins().await {
            Ok(origins) => origins,
            Err(error) => {
                tracing::warn!(%error, "web: linked origins unavailable");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    let path = uri.path().trim_start_matches('/');
    let mut response = None;
    if path == "index.html" || !path.ends_with(".html") {
        if let Some((_, bytes)) = embedded::ASSETS.iter().find(|(asset, _)| *asset == path) {
            response = Some(asset_response(path, bytes));
        }
    }
    if response.is_none() && is_client_route(uri.path()) && !is_asset_like(uri.path()) {
        if let Some((_, index)) = embedded::ASSETS
            .iter()
            .find(|(asset, _)| *asset == "index.html")
        {
            response = Some(asset_response("index.html", index));
        }
    }
    let mut response = response.unwrap_or_else(|| {
        protected_response(StatusCode::NOT_FOUND, Body::empty(), None, NO_CACHE)
    });
    if let Ok(policy) = HeaderValue::from_str(&content_security_policy(&linked)) {
        response
            .headers_mut()
            .insert(header::CONTENT_SECURITY_POLICY, policy);
    }
    response
}

/// The page's full policy: the build's script and style rules, plus where it may
/// connect and load media from. Those live only in this header, not the page's
/// meta tag, because browsers enforce both and the meta tag can't know which
/// servers users have linked. Exact origins rather than `https:`: injected
/// script could otherwise send a token anywhere. `linked` holds origins that
/// passed validation when they were linked.
fn content_security_policy(linked: &[String]) -> String {
    let origins = linked
        .iter()
        .fold(String::new(), |all, origin| format!("{all} {origin}"));
    format!(
        "default-src 'self'; connect-src 'self'{origins}; img-src 'self' blob:{origins}; \
         media-src 'self' blob:{origins}; {}",
        embedded::CONTENT_SECURITY_POLICY
    )
}

/// CORS for clients connected from other origins. Requests authenticate with
/// bearer tokens and media tokens rather than cookies, so a foreign page gets no
/// ambient authority. Refused until the server is set up: otherwise any website
/// a visitor opened could finish setup and take the admin account.
pub fn cross_origin_layer(storage: Arc<dyn Storage>) -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::async_predicate(move |_origin, _parts| {
            let storage = storage.clone();
            async move {
                storage
                    .list_users()
                    .await
                    .is_ok_and(|users| !users.is_empty())
            }
        }))
        .allow_methods([Method::GET, Method::POST, Method::HEAD])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::RANGE])
        .expose_headers([
            header::CONTENT_RANGE,
            header::ACCEPT_RANGES,
            header::CONTENT_LENGTH,
        ])
        .allow_private_network(true)
}

fn is_asset_like(path: &str) -> bool {
    path.starts_with("/_app/")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|segment| segment.contains('.'))
}

fn is_client_route(path: &str) -> bool {
    matches!(
        path,
        "/" | "/browse"
            | "/libraries"
            | "/search"
            | "/collections"
            | "/live-tv"
            | "/watch-rooms"
            | "/account"
            | "/admin"
            | "/invite"
    ) || has_single_parameter(path, "/browse/")
        || has_single_parameter(path, "/libraries/")
        || has_single_parameter(path, "/works/")
}

fn has_single_parameter(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|parameter| !parameter.is_empty() && !parameter.contains('/'))
}

fn asset_response(path: &str, bytes: &'static [u8]) -> Response {
    let cache_control = if path.starts_with("_app/immutable/") {
        IMMUTABLE
    } else {
        NO_CACHE
    };
    let mut response = protected_response(
        StatusCode::OK,
        Body::from(bytes),
        Some(content_type(path)),
        cache_control,
    );
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(bytes.len()));
    response
}

fn protected_response(
    status: StatusCode,
    body: Body,
    content_type: Option<&'static str>,
    cache_control: &'static str,
) -> Response {
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    if let Some(content_type) = content_type {
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(embedded::CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json; charset=utf-8",
        Some("webmanifest") => "application/manifest+json; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        routing::get,
    };
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn serves_shell_for_canonical_client_routes() {
        let app = routes::<()>();
        for path in ["/", "/libraries", "/libraries/abc", "/works/abc", "/admin"] {
            let response = app
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/html; charset=utf-8"
            );
            assert_eq!(response.headers()[header::CACHE_CONTROL], NO_CACHE);
            assert_protections(&response);
        }
    }

    #[tokio::test]
    async fn serves_nested_immutable_assets_with_mime_and_cache_headers() {
        let Some((path, _)) = embedded::ASSETS.iter().find(|(path, _)| {
            path.starts_with("_app/immutable/") && (path.ends_with(".js") || path.ends_with(".css"))
        }) else {
            return;
        };

        let response = routes::<()>()
            .oneshot(
                Request::get(format!("/{path}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], IMMUTABLE);
        assert_eq!(response.headers()[header::CONTENT_TYPE], content_type(path));
        assert_protections(&response);
    }

    #[tokio::test]
    async fn exact_api_route_wins_and_unknown_assets_are_not_spa_fallbacks() {
        let app = routes::<()>().route("/graphql", get(|| async { "api" }));
        let api = app
            .clone()
            .oneshot(Request::get("/graphql").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(to_bytes(api.into_body(), usize::MAX).await.unwrap(), "api");

        for path in [
            "/missing.js",
            "/_app/immutable/missing.js",
            "/libraries/missing.js",
            "/account.html",
            "/libraries/one/two",
            "/works",
            "/not-a-client-route",
        ] {
            let response = app
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn head_returns_headers_without_a_body() {
        let response = routes::<()>()
            .oneshot(Request::head("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        assert!(
            response.headers().contains_key(header::CONTENT_LENGTH),
            "HEAD must retain the GET content length"
        );
        assert!(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn header_csp_matches_sveltekit_hash_mode_output() {
        let index = embedded::ASSETS
            .iter()
            .find(|(path, _)| *path == "index.html")
            .unwrap()
            .1;
        let index = std::str::from_utf8(index).unwrap();
        let Some(svelte_csp) = index
            .split("http-equiv=\"content-security-policy\" content=\"")
            .nth(1)
            .and_then(|tail| tail.split('\"').next())
        else {
            return;
        };

        for directive in [
            "object-src",
            "script-src",
            "style-src",
            "base-uri",
            "form-action",
        ] {
            let expected = svelte_csp
                .split(';')
                .map(str::trim)
                .find(|value| value.starts_with(directive))
                .unwrap();
            assert!(
                embedded::CONTENT_SECURITY_POLICY
                    .split(';')
                    .map(str::trim)
                    .any(|value| value == expected),
                "missing or altered {directive}"
            );
        }
        // Browsers enforce the meta tag too, so anything here would block
        // linked servers no matter what the header says.
        for directive in ["default-src", "connect-src", "img-src", "media-src"] {
            assert!(
                !svelte_csp.contains(directive),
                "{directive} must stay out of the meta tag"
            );
        }
    }

    #[test]
    fn linked_origins_are_allowed_exactly() {
        let policy = content_security_policy(&["https://friend.example".to_string()]);
        for directive in [
            "connect-src 'self' https://friend.example",
            "img-src 'self' blob: https://friend.example",
            "media-src 'self' blob: https://friend.example",
        ] {
            assert!(policy.contains(directive), "{policy}");
        }
        assert!(policy.contains("default-src 'self'"), "{policy}");
        assert!(!policy.contains("https:;"), "{policy}");
        assert!(
            !content_security_policy(&[]).contains("friend"),
            "nothing linked, nothing allowed"
        );
    }

    #[tokio::test]
    async fn cross_origin_requests_wait_for_setup() {
        use noflex_storage::domain::User;
        use noflex_storage_turso::TursoStore;
        use tower::ServiceExt;

        let store = Arc::new(TursoStore::open_temp().await.expect("store"));
        store.migrate().await.expect("migrate");
        let app = Router::new()
            .route("/graphql", axum::routing::post(|| async { "ok" }))
            .layer(cross_origin_layer(store.clone()));
        let preflight = || {
            axum::http::Request::builder()
                .method(Method::OPTIONS)
                .uri("/graphql")
                .header(header::ORIGIN, "https://evil.example")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
                .body(Body::empty())
                .unwrap()
        };

        let response = app.clone().oneshot(preflight()).await.unwrap();
        assert!(
            !response
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            "another site must not reach the setup wizard"
        );

        store
            .upsert_user(&User {
                id: noflex_core::UserId::new(),
                name: "admin".to_string(),
                password_hash: "argon2id$placeholder".to_string(),
                is_admin: true,
                role: noflex_core::UserRole::Admin,
                created_at: time::OffsetDateTime::now_utc(),
            })
            .await
            .expect("user");
        let response = app.oneshot(preflight()).await.unwrap();
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://evil.example",
            "after setup every origin may try, with its own credentials"
        );
        assert!(
            !response
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS),
            "no ambient authority"
        );
    }

    /// Every SvelteKit page must be served the shell on a deep link.
    /// `[param]` segments stand for one path parameter; `(group)`
    /// segments don't appear in URLs.
    #[test]
    fn client_route_allowlist_covers_every_sveltekit_page() {
        fn walk(dir: &std::path::Path, segments: &mut Vec<String>, routes: &mut Vec<String>) {
            let mut entries = std::fs::read_dir(dir)
                .expect("read routes directory")
                .map(|entry| entry.expect("route entry"))
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().expect("route entry type").is_dir() {
                    let segment = if name.starts_with('(') && name.ends_with(')') {
                        None
                    } else if name.starts_with("[[") || name.starts_with("[...") {
                        panic!("route {name} needs an allowlist rule shape this test doesn't model")
                    } else if name.starts_with('[') && name.ends_with(']') {
                        Some("0193b0c4-5e7a-7000-8000-000000000000".to_owned())
                    } else {
                        Some(name)
                    };
                    let pushed = segment.map(|segment| segments.push(segment)).is_some();
                    walk(&entry.path(), segments, routes);
                    if pushed {
                        segments.pop();
                    }
                } else if name.starts_with("+page.") {
                    routes.push(format!("/{}", segments.join("/")));
                }
            }
        }

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/routes");
        if !root.is_dir() {
            eprintln!("skipping: {} not present", root.display());
            return;
        }
        let mut routes = Vec::new();
        walk(&root, &mut Vec::new(), &mut routes);
        routes.dedup();
        assert!(!routes.is_empty(), "no SvelteKit pages found");
        for route in routes {
            assert!(
                is_client_route(&route) && !is_asset_like(&route),
                "SvelteKit page {route} is missing from is_client_route"
            );
        }
    }

    #[test]
    fn development_fallback_is_embedded_when_build_is_absent() {
        if !embedded::WEB_BUILD_PRESENT {
            let index = embedded::ASSETS
                .iter()
                .find(|(path, _)| *path == "index.html")
                .unwrap()
                .1;
            assert!(String::from_utf8_lossy(index).contains("web application has not been built"));
        }
    }

    fn assert_protections(response: &Response) {
        let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap();
        assert!(!csp.contains("'unsafe-inline'"));
        assert!(!csp.contains("'unsafe-hashes'"));
        assert!(!csp.contains("data:"));
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
    }
}
