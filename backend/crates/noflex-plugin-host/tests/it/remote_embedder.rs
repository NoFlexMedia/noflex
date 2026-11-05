// SPDX-License-Identifier: AGPL-3.0-only
//
// The OpenAI-compatible embeddings client against a stand-in server: what goes
// over the wire, what comes back, and which settings force a reindex.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use noflex_plugin_host::{EmbedDocument, EmbedInput, RemoteEmbedError, RemoteEmbedder};
use noflex_storage::domain::EmbeddingEndpoint;
use serde_json::{Value, json};

/// The auth header and body of the last request.
type Seen = Arc<Mutex<Option<(Option<String>, Value)>>>;

fn endpoint(url: &str) -> EmbeddingEndpoint {
    EmbeddingEndpoint {
        url: url.to_string(),
        api_key: Some("sk-test".to_string()),
        model: "embed-small".to_string(),
        dimensions: 2,
        request_dimensions: None,
        query_prefix: "query: ".to_string(),
        document_prefix: "passage: ".to_string(),
    }
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{address}/v1")
}

fn document(title: &str, text: &str) -> EmbedInput {
    EmbedInput::Document(EmbedDocument {
        title: title.to_string(),
        text: text.to_string(),
    })
}

#[tokio::test]
async fn sends_the_standard_request_and_orders_and_normalizes_the_answer() {
    let seen = Arc::new(Mutex::new(None));
    let app = Router::new()
        .route(
            "/v1/embeddings",
            post(
                |State(seen): State<Seen>, headers: HeaderMap, Json(body): Json<Value>| async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    *seen.lock().expect("lock") = Some((auth, body));
                    // Servers may answer out of order, and not at unit length.
                    Json(json!({ "data": [
                        { "index": 1, "embedding": [0.0, 5.0] },
                        { "index": 0, "embedding": [3.0, 4.0] },
                    ]}))
                },
            ),
        )
        .with_state(seen.clone());
    let url = serve(app).await;
    // A pasted full path and trailing slash land on the same route.
    let mut config = endpoint(&format!("{url}/embeddings/"));
    config.request_dimensions = Some(2);
    let embedder = RemoteEmbedder::new(config).expect("client");

    let vectors = embedder
        .embed(&[document("Alien (1979)", "In space."), document("Heat", "")])
        .await
        .expect("embed");

    assert_eq!(vectors, [vec![0.6, 0.8], vec![0.0, 1.0]]);
    let (auth, body) = seen.lock().expect("lock").take().expect("a request");
    assert_eq!(auth.as_deref(), Some("Bearer sk-test"));
    assert_eq!(body["model"], "embed-small");
    assert_eq!(body["dimensions"], 2);
    assert_eq!(
        body["input"],
        json!(["passage: Alien (1979). In space.", "passage: Heat"])
    );
}

#[tokio::test]
async fn document_batches_retry_rate_limits_but_queries_do_not() {
    let calls = Arc::new(AtomicU32::new(0));
    let app = Router::new()
        .route(
            "/v1/embeddings",
            post(|State(calls): State<Arc<AtomicU32>>| async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    return (
                        StatusCode::TOO_MANY_REQUESTS,
                        [("retry-after", "1")],
                        Json(json!({ "error": { "message": "slow down" } })),
                    );
                }
                (
                    StatusCode::OK,
                    [("retry-after", "0")],
                    Json(json!({ "data": [{ "index": 0, "embedding": [1.0, 0.0] }] })),
                )
            }),
        )
        .with_state(calls.clone());
    let embedder = RemoteEmbedder::new(endpoint(&serve(app).await)).expect("client");

    let query = embedder
        .embed(&[EmbedInput::Query("alien".to_string())])
        .await
        .expect_err("a search falls back rather than wait");
    assert!(query.to_string().contains("slow down"), "{query}");

    calls.store(0, Ordering::SeqCst);
    embedder
        .embed(&[document("Alien", "")])
        .await
        .expect("retried past the rate limit");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn rejects_vectors_that_do_not_match_the_measured_size() {
    let app = Router::new().route(
        "/v1/embeddings",
        post(|| async { Json(json!({ "data": [{ "index": 0, "embedding": [1.0, 0.0, 0.0] }] })) }),
    );
    let embedder = RemoteEmbedder::new(endpoint(&serve(app).await)).expect("client");
    let error = embedder
        .embed(&[document("Alien", "")])
        .await
        .expect_err("3 dimensions where 2 were measured");
    assert!(matches!(error, RemoteEmbedError::Malformed(_)), "{error}");
}

#[tokio::test]
async fn probe_measures_the_model_and_reports_why_it_failed() {
    let good = Router::new().route(
        "/v1/embeddings",
        post(|| async { Json(json!({ "data": [{ "index": 0, "embedding": [1.0, 0.0, 0.0] }] })) }),
    );
    let measured = RemoteEmbedder::probe(endpoint(&serve(good).await))
        .await
        .expect("probe");
    assert_eq!(measured.dimensions, 3);

    let denied = Router::new().route(
        "/v1/embeddings",
        post(|| async {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": { "message": "bad key" } })),
            )
        }),
    );
    let error = RemoteEmbedder::probe(endpoint(&serve(denied).await))
        .await
        .expect_err("rejected");
    let message = error.to_string();
    assert!(
        message.contains("401") && message.contains("bad key"),
        "{message}"
    );
    assert!(!message.contains("sk-test"), "{message}");
}

#[test]
fn only_what_changes_the_vectors_changes_the_namespace() {
    let namespace = |config: EmbeddingEndpoint| {
        RemoteEmbedder::new(config)
            .expect("client")
            .claim()
            .namespace
            .clone()
    };
    let base = endpoint("https://a.example/v1");
    let original = namespace(base.clone());

    let mut rotated = base.clone();
    rotated.api_key = Some("sk-rotated".to_string());
    assert_eq!(namespace(rotated), original, "a new key isn't a new model");
    let mut slashed = base.clone();
    slashed.url.push('/');
    assert_eq!(namespace(slashed), original);

    let changes: [fn(&mut EmbeddingEndpoint); 5] = [
        |config| config.url = "https://b.example/v1".to_string(),
        |config| config.model = "embed-large".to_string(),
        |config| config.dimensions = 3,
        |config| config.query_prefix = "q: ".to_string(),
        |config| config.document_prefix = "d: ".to_string(),
    ];
    for change in changes {
        let mut changed = base.clone();
        change(&mut changed);
        assert_ne!(namespace(changed), original);
    }
}
