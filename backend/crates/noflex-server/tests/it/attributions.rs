// SPDX-License-Identifier: AGPL-3.0-only
//
// The public logo route: no credential, and an SVG that can't script this origin.

use std::sync::Arc;

use axum::{
    Extension,
    body::Body,
    http::{Request, StatusCode, header},
};
use noflex_storage::{Storage, domain::ProviderAttribution};
use noflex_storage_turso::TursoStore;
use tower::ServiceExt;

const SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>";

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn logos_are_public_cached_and_sandboxed() {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let storage: Arc<dyn Storage> = Arc::new(store);
    storage
        .upsert_provider_attribution(
            &ProviderAttribution {
                provider: "tmdb".into(),
                name: "TMDB".into(),
                text: "Data from TMDB.".into(),
                url: None,
                logo_mime: Some("image/svg+xml".into()),
                logo_digest: Some("d1".into()),
            },
            Some(SVG),
        )
        .await
        .expect("attribution");
    storage
        .upsert_provider_attribution(
            &ProviderAttribution {
                provider: "bare".into(),
                name: "Bare".into(),
                text: "Data from Bare.".into(),
                url: None,
                logo_mime: None,
                logo_digest: None,
            },
            None,
        )
        .await
        .expect("attribution");
    let app = noflex_server::attributions::routes::<()>().layer(Extension(storage));

    let response = app
        .clone()
        .oneshot(get("/attributions/tmdb/logo?v=d1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers[header::CONTENT_TYPE], "image/svg+xml");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(
        headers[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
    let policy = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(policy.contains("sandbox"), "{policy}");
    assert!(policy.contains("default-src 'none'"), "{policy}");
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], SVG);

    for path in ["/attributions/bare/logo", "/attributions/ghost/logo"] {
        let response = app.clone().oneshot(get(path)).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}
