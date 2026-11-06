// SPDX-License-Identifier: AGPL-3.0-only
//
// End-to-end checks for the GraphQL WebSocket endpoint: a real socket,
// the graphql-transport-ws handshake, and in-band authentication.

use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use noflex_auth::{ClientAddr, JwtConfig, issue_jwt};
use noflex_core::{UserId, UserRole};
use noflex_graphql::{SchemaContext, build_schema};
use noflex_storage::{Storage, domain::User};
use noflex_storage_turso::TursoStore;
use time::OffsetDateTime;
use tokio_tungstenite::tungstenite::{
    Message,
    client::IntoClientRequest,
    http::{HeaderValue, header},
};

const JWT_SECRET: &[u8] = b"graphql-websocket-tests-secret-32b";

/// graphql-transport-ws "Forbidden": the client must re-authenticate.
const INIT_REFUSED: u16 = 4403;

struct Server {
    url: String,
    token: String,
}

async fn server() -> Server {
    let store = TursoStore::open_temp().await.expect("store");
    store.migrate().await.expect("migrate");
    let store = Arc::new(store);
    let jwt = Arc::new(JwtConfig::dev(JWT_SECRET).expect("jwt"));
    let user = User {
        id: UserId::new(),
        username: "viewer".into(),
        display_name: None,
        password_hash: String::new(),
        is_admin: false,
        role: UserRole::User,
        created_at: OffsetDateTime::now_utc(),
        avatar_updated_at: None,
    };
    store.upsert_user(&user).await.expect("user");
    let now = OffsetDateTime::now_utc();
    let session = noflex_storage::domain::Session {
        id: noflex_core::SessionId::new(),
        user_id: user.id,
        client: None,
        device: None,
        device_id: None,
        app_version: None,
        remote_addr: None,
        created_at: now,
        last_seen_at: now,
        expires_at: now + time::Duration::days(1),
        refresh_token_hash: None,
    };
    store.create_session(&session).await.expect("session");
    let token = issue_jwt(&jwt, user.id, false, session.id).expect("token");
    let storage: Arc<dyn Storage> = store.clone();
    let schema = build_schema(SchemaContext {
        storage: Some(store),
        jwt: Some(jwt.clone()),
        ..Default::default()
    });
    let app = axum::Router::new().route(
        "/graphql/ws",
        axum::routing::get(
            move |protocol: async_graphql_axum::GraphQLProtocol,
                  headers: axum::http::HeaderMap,
                  ws: axum::extract::ws::WebSocketUpgrade| async move {
                noflex_server::graphql_ws::upgrade(
                    ws,
                    protocol,
                    schema,
                    jwt,
                    storage,
                    &headers,
                    ClientAddr(std::net::Ipv4Addr::LOCALHOST.into()),
                )
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await });
    Server {
        url: format!("ws://{addr}/graphql/ws"),
        token,
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(url: &str, authorization: Option<&str>) -> Socket {
    let mut request = url.into_client_request().expect("request");
    request.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("graphql-transport-ws"),
    );
    if let Some(value) = authorization {
        request
            .headers_mut()
            .insert(header::AUTHORIZATION, value.parse().expect("header"));
    }
    tokio_tungstenite::connect_async(request)
        .await
        .expect("connect")
        .0
}

async fn send(socket: &mut Socket, value: serde_json::Value) {
    socket
        .send(Message::text(value.to_string()))
        .await
        .expect("send");
}

/// The next protocol frame: a JSON message, or `{"close": code}`.
async fn next(socket: &mut Socket) -> serde_json::Value {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .expect("frame in time");
    match frame {
        Some(Ok(Message::Text(text))) => serde_json::from_str(&text).expect("json"),
        Some(Ok(Message::Close(close))) => {
            serde_json::json!({ "close": close.map(|c| u16::from(c.code)) })
        }
        Some(Ok(_)) => Box::pin(next(socket)).await,
        Some(Err(_)) | None => serde_json::json!({ "close": null }),
    }
}

#[tokio::test]
async fn init_without_credential_is_refused() {
    let server = server().await;
    let mut socket = connect(&server.url, None).await;
    send(
        &mut socket,
        serde_json::json!({ "type": "connection_init" }),
    )
    .await;
    assert_eq!(next(&mut socket).await["close"], INIT_REFUSED);
}

#[tokio::test]
async fn init_with_bad_token_is_refused() {
    let server = server().await;
    let mut socket = connect(&server.url, None).await;
    send(
        &mut socket,
        serde_json::json!({ "type": "connection_init", "payload": { "authorization": "Bearer nope" } }),
    )
    .await;
    assert_eq!(next(&mut socket).await["close"], INIT_REFUSED);
}

#[tokio::test]
async fn payload_token_authenticates_and_subscriptions_run() {
    let server = server().await;
    let mut socket = connect(&server.url, None).await;
    send(
        &mut socket,
        serde_json::json!({
            "type": "connection_init",
            "payload": { "authorization": format!("Bearer {}", server.token) },
        }),
    )
    .await;
    assert_eq!(next(&mut socket).await["type"], "connection_ack");

    // The resolver runs as the authenticated viewer: an unknown room is
    // a resolver error, not an authentication error.
    send(
        &mut socket,
        serde_json::json!({
            "id": "1",
            "type": "subscribe",
            "payload": {
                "query": "subscription($id: ID!) { watchRoomEvents(roomId: $id) { action } }",
                "variables": { "id": uuid::Uuid::now_v7().to_string() },
            },
        }),
    )
    .await;
    let reply = next(&mut socket).await;
    assert_eq!(reply["id"], "1");
    let text = reply.to_string();
    assert!(text.contains("watch room not found"), "{text}");
    assert!(!text.contains("authentication"), "{text}");
}

#[tokio::test]
async fn upgrade_headers_authenticate_when_payload_is_empty() {
    let server = server().await;
    let bearer = format!("Bearer {}", server.token);
    let mut socket = connect(&server.url, Some(&bearer)).await;
    send(
        &mut socket,
        serde_json::json!({ "type": "connection_init" }),
    )
    .await;
    assert_eq!(next(&mut socket).await["type"], "connection_ack");
}
