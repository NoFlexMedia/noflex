// SPDX-License-Identifier: AGPL-3.0-only
//
// GraphQL subscriptions over WebSocket (`graphql-transport-ws`, plus the
// legacy `graphql-ws` subprotocol). Browsers can't set headers on an
// upgrade request, so the credential travels in the `connection_init`
// payload instead; non-browser clients may still authenticate the upgrade
// itself with the usual headers. Auth is in-band rather than cookie-based,
// so a cross-site page can open the socket but never authenticate it.

use std::{sync::Arc, time::Duration};

use async_graphql::Data;
use async_graphql_axum::{GraphQLProtocol, GraphQLWebSocket};
use axum::{
    extract::ws::{CloseFrame, Message, WebSocketUpgrade},
    http::{HeaderMap, HeaderName, HeaderValue, header},
    response::Response,
};
use futures::{SinkExt, StreamExt, future};
use noflex_auth::{ClientAddr, JwtConfig, Principal, principal_from_headers};
use noflex_graphql::NoflexSchema;
use noflex_storage::{CallerSession, RequestUser, Storage};
use tokio::sync::oneshot;

/// A connection that hasn't completed `connection_init` by then is
/// closed (the protocol's 4408 case).
const INIT_TIMEOUT: Duration = Duration::from_secs(10);

/// The refusal message our `connection_init` handler returns; the close
/// frame carrying it is rewritten from async-graphql's generic 1002 to
/// the protocol's 4403 so clients can tell "sign in again" from a bug.
const AUTH_REQUIRED: &str = "authentication required";

/// How often a live connection's credential is re-resolved. Bounds how
/// long a revoked session, deleted user, expired access token, or role
/// change keeps receiving events; clients reconnect with a fresh token.
const REVALIDATE_EVERY: Duration = Duration::from_secs(30);

/// Upgrade to a GraphQL WebSocket. `upgrade_headers` are the upgrade
/// request's headers, used as the credential when `connection_init`
/// carries none. A connection whose credential doesn't resolve to a
/// user is refused at `connection_init`; one whose credential stops
/// resolving to the same user and role is closed.
pub fn upgrade(
    ws: WebSocketUpgrade,
    protocol: GraphQLProtocol,
    schema: NoflexSchema,
    jwt: Arc<JwtConfig>,
    storage: Arc<dyn Storage>,
    upgrade_headers: &HeaderMap,
    peer: ClientAddr,
) -> Response {
    let fallback = credential_only(upgrade_headers);
    ws.protocols(async_graphql::http::ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |socket| async move {
            let (accepted_tx, accepted_rx) = oneshot::channel();
            let init = {
                let jwt = jwt.clone();
                let storage = storage.clone();
                move |payload: serde_json::Value| async move {
                    let credential = payload_credential(&payload).unwrap_or(fallback);
                    let principal =
                        principal_from_headers(&credential, &jwt, storage.as_ref()).await;
                    if !principal.is_authenticated() {
                        return Err(async_graphql::Error::new(AUTH_REQUIRED));
                    }
                    let _ = accepted_tx.send((credential, principal));
                    let mut data = Data::default();
                    data.insert(principal);
                    data.insert(peer);
                    data.insert(RequestUser {
                        user_id: principal.user_id,
                        is_admin: principal.is_admin,
                    });
                    if let Some(session) = principal.session_id {
                        data.insert(CallerSession(session));
                    }
                    Ok(data)
                }
            };
            let (sink, stream) = socket.split();
            let sink = sink.with(|message| future::ready(Ok::<_, axum::Error>(forbid(message))));
            let serve = GraphQLWebSocket::new_with_pair(sink, stream, schema, protocol)
                .on_connection_init(init)
                .serve();
            tokio::select! {
                () = serve => {}
                () = supervise(accepted_rx, jwt, storage) => {}
            }
        })
}

/// Resolves when the connection must be dropped: no successful init in
/// time, or the accepted credential no longer maps to the same principal.
/// A failed init never resolves here — the protocol layer closes that
/// socket itself with the error.
async fn supervise(
    accepted: oneshot::Receiver<(HeaderMap, Principal)>,
    jwt: Arc<JwtConfig>,
    storage: Arc<dyn Storage>,
) {
    let (credential, principal) = match tokio::time::timeout(INIT_TIMEOUT, accepted).await {
        Err(_) => return,
        Ok(Err(_)) => return std::future::pending().await,
        Ok(Ok(accepted)) => accepted,
    };
    let mut tick = tokio::time::interval(REVALIDATE_EVERY);
    tick.tick().await;
    loop {
        tick.tick().await;
        let current = principal_from_headers(&credential, &jwt, storage.as_ref()).await;
        if current.user_id != principal.user_id
            || current.is_admin != principal.is_admin
            || !current.is_authenticated()
        {
            tracing::debug!(user = %principal.user_id, "closing GraphQL websocket: credential no longer valid");
            return;
        }
    }
}

/// A refused-init close becomes 4403 Forbidden; everything else passes.
fn forbid(message: Message) -> Message {
    match message {
        Message::Close(Some(frame)) if frame.code == 1002 && frame.reason == AUTH_REQUIRED => {
            Message::Close(Some(CloseFrame {
                code: 4403,
                reason: frame.reason,
            }))
        }
        other => other,
    }
}

/// The credential headers of `headers`, so the supervisor doesn't hold
/// the whole upgrade request.
fn credential_only(headers: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for name in [
        header::AUTHORIZATION,
        HeaderName::from_static("x-api-key"),
        HeaderName::from_static("x-emby-token"),
    ] {
        if let Some(value) = headers.get(&name) {
            out.insert(name, value.clone());
        }
    }
    out
}

/// Map a `connection_init` payload onto credential headers. Accepts
/// `{"authorization": "Bearer …"}` (any key case), `{"token": "…"}` and
/// `{"apiKey": "…"}`. `None` when the payload carries no credential.
fn payload_credential(payload: &serde_json::Value) -> Option<HeaderMap> {
    let object = payload.as_object()?;
    let field = |wanted: &str| {
        object
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(wanted))
            .and_then(|(_, value)| value.as_str())
    };
    let mut headers = HeaderMap::new();
    if let Some(auth) = field("authorization") {
        headers.insert(header::AUTHORIZATION, HeaderValue::from_str(auth).ok()?);
    } else if let Some(token) = field("token") {
        let value = HeaderValue::from_str(&format!("Bearer {token}")).ok()?;
        headers.insert(header::AUTHORIZATION, value);
    } else if let Some(key) = field("apiKey").or_else(|| field("x-api-key")) {
        headers.insert("x-api-key", HeaderValue::from_str(key).ok()?);
    } else {
        return None;
    }
    Some(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_credential_shapes() {
        let bearer = payload_credential(&serde_json::json!({"Authorization": "Bearer abc"}))
            .expect("authorization");
        assert_eq!(bearer[header::AUTHORIZATION], "Bearer abc");

        let token = payload_credential(&serde_json::json!({"token": "abc"})).expect("token");
        assert_eq!(token[header::AUTHORIZATION], "Bearer abc");

        let key = payload_credential(&serde_json::json!({"apiKey": "nfk_1"})).expect("api key");
        assert_eq!(key["x-api-key"], "nfk_1");

        assert!(payload_credential(&serde_json::json!({})).is_none());
        assert!(payload_credential(&serde_json::Value::Null).is_none());
        assert!(payload_credential(&serde_json::json!({"token": "bad\nvalue"})).is_none());
    }

    #[test]
    fn credential_only_keeps_auth_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer x"));
        headers.insert(header::COOKIE, HeaderValue::from_static("a=b"));
        let out = credential_only(&headers);
        assert_eq!(out.len(), 1);
        assert!(out.contains_key(header::AUTHORIZATION));
    }
}
