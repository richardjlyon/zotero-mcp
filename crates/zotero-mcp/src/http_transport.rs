//! HTTP/SSE transport for `zotero-mcp`.
//!
//! Mirrors the MCP-over-SSE protocol that rmcp's `SseServer` implements (so
//! existing MCP clients connect with no additional config), but mounts the
//! router ourselves so we can layer a bearer-token check in front of both
//! routes. rmcp 0.1.5's `SseServer::serve` builds its own router internally
//! and exposes no hook for middleware, hence this small reimplementation.
//!
//! Routes:
//!   - `GET /sse`            — server-sent events stream. First event is an
//!                             `endpoint` event whose data is the per-session
//!                             POST URL (`/message?sessionId=<hex>`).
//!   - `POST /message?...`   — client → server JSON-RPC messages keyed by
//!                             `sessionId`.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, Request, State},
    http::StatusCode,
    response::{IntoResponse, sse::{Event, KeepAlive, Sse}},
    routing::{get, post},
};
use futures::{SinkExt, StreamExt};
use rmcp::{
    ServiceExt,
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::PollSender;
use tower_http::validate_request::ValidateRequestHeaderLayer;

use crate::oauth::{self, OAuthState};
use crate::server::ZoteroServer;
use crate::state::AppState;

type SessionId = Arc<str>;
type ClientTx = tokio::sync::mpsc::Sender<ClientJsonRpcMessage>;
type TxStore = Arc<tokio::sync::RwLock<HashMap<SessionId, ClientTx>>>;

#[derive(Clone)]
struct AppShared {
    state: AppState,
    txs: TxStore,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostQuery {
    session_id: String,
}

async fn post_handler(
    State(app): State<AppShared>,
    Query(q): Query<PostQuery>,
    Json(msg): Json<ClientJsonRpcMessage>,
) -> Result<StatusCode, StatusCode> {
    let tx = {
        let g = app.txs.read().await;
        g.get(q.session_id.as_str())
            .ok_or(StatusCode::NOT_FOUND)?
            .clone()
    };
    if tx.send(msg).await.is_err() {
        return Err(StatusCode::GONE);
    }
    Ok(StatusCode::ACCEPTED)
}

async fn sse_handler(
    State(app): State<AppShared>,
) -> Sse<impl futures::Stream<Item = Result<Event, std::io::Error>>> {
    let session: SessionId = Arc::from(format!("{:016x}", rand::random::<u128>()));
    tracing::info!(%session, "new SSE session");

    let (from_client_tx, from_client_rx) = tokio::sync::mpsc::channel::<ClientJsonRpcMessage>(64);
    let (to_client_tx, to_client_rx) = tokio::sync::mpsc::channel::<ServerJsonRpcMessage>(64);
    app.txs
        .write()
        .await
        .insert(session.clone(), from_client_tx);

    // Spawn a ZoteroServer instance for this session. rmcp's `IntoTransport`
    // impl for `(Sink, Stream)` lets us hand it a tuple of mpsc-backed adapters.
    let session_for_cleanup = session.clone();
    let txs_for_cleanup = app.txs.clone();
    let state = app.state.clone();
    tokio::spawn(async move {
        let stream = ReceiverStream::new(from_client_rx);
        // rmcp's serve() requires the transport's sink error type to satisfy
        // `From<std::io::Error>`. PollSender's native error doesn't, so we
        // coerce it via sink_map_err.
        let sink = PollSender::new(to_client_tx).sink_map_err(std::io::Error::other);
        let transport = (sink, stream);
        let server = ZoteroServer::new(state);
        match server.serve(transport).await {
            Ok(running) => {
                if let Err(e) = running.waiting().await {
                    tracing::warn!(error = %e, "SSE session ended with error");
                }
            }
            Err(e) => tracing::error!(error = %e, "SSE service init error"),
        }
        txs_for_cleanup.write().await.remove(&session_for_cleanup);
        tracing::info!(%session_for_cleanup, "SSE session cleaned up");
    });

    // Cloudflare quick-tunnels buffer small SSE responses at the HTTP/2 edge
    // until ~2 KB of body has accumulated, which delays the `endpoint` event by
    // up to the keep-alive interval. Prefix a ~2 KB comment so the very first
    // chunk forces the edge buffer to flush.
    let session_for_endpoint = session.clone();
    let padding = ":".to_string() + &"x".repeat(2048);
    let init = futures::stream::iter(vec![
        Ok::<_, std::io::Error>(Event::default().comment(padding)),
        Ok(Event::default()
            .event("endpoint")
            .data(format!("/message?sessionId={session_for_endpoint}"))),
    ]);
    let rest = ReceiverStream::new(to_client_rx).map(|m| match serde_json::to_string(&m) {
        Ok(s) => Ok(Event::default().event("message").data(s)),
        Err(e) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
    });
    Sse::new(init.chain(rest))
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(5)))
}

pub async fn run(
    state: AppState,
    addr: SocketAddr,
    bearer: Option<String>,
    oauth_state: Option<OAuthState>,
) -> anyhow::Result<()> {
    let shared = AppShared {
        state,
        txs: Default::default(),
    };
    let mut resource_routes = Router::new()
        .route("/sse", get(sse_handler))
        .route("/message", post(post_handler))
        .with_state(shared);

    // OAuth bearer-token enforcement only applies to the resource routes
    // (/sse, /message). Discovery and /oauth/token must stay reachable
    // unauthenticated for the OAuth handshake itself to work.
    if let Some(oauth_state) = oauth_state.clone() {
        resource_routes = resource_routes
            .layer(axum::middleware::from_fn_with_state(
                oauth_state,
                require_bearer_token,
            ));
    }

    let mut app = Router::new().merge(resource_routes);
    if let Some(oauth_state) = oauth_state {
        app = app.merge(oauth::router(oauth_state));
        tracing::info!("OAuth 2.1 surface mounted (discovery + /oauth/token + bearer gate on /sse, /message)");
    }
    let mut app = app.layer(tower_http::trace::TraceLayer::new_for_http());

    if let Some(token) = bearer {
        // Legacy shared-secret bearer auth, preserved for environments that
        // still pin a single static token via ZOTERO_MCP_BEARER_TOKEN. Applies
        // to every route (including discovery), so do not combine with the
        // OAuth flow.
        #[allow(deprecated)]
        let auth = ValidateRequestHeaderLayer::bearer(&token);
        app = app.layer(auth);
        tracing::info!(%addr, "zotero-mcp HTTP/SSE transport listening (static bearer auth enabled)");
    } else {
        tracing::warn!(
            %addr,
            "zotero-mcp HTTP/SSE transport listening WITHOUT static bearer — \
             OAuth gates /sse and /message if configured; otherwise transport-level access control applies"
        );
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Per-request guard for the resource routes. Reads the `Authorization`
/// header, validates the bearer token against the in-memory token store, and
/// either passes the request through or returns `401 Unauthorized` with a
/// `WWW-Authenticate` challenge that points clients at the resource metadata
/// document (RFC 9728 §5.1). On failure clients are expected to fetch
/// `resource_metadata`, walk to the advertised authorization server, and call
/// `/oauth/token` to acquire a token.
async fn require_bearer_token(
    axum::extract::State(oauth_state): axum::extract::State<OAuthState>,
    req: Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let bearer = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));

    if let Some(token) = bearer {
        if oauth_state.validate_token(token.trim()).await {
            return next.run(req).await;
        }
    }

    let challenge = format!(
        "Bearer realm=\"zotero-mcp\", resource_metadata=\"{}\", scope=\"mcp\"",
        oauth_state.resource_metadata_url()
    );
    let (status, error) = if bearer.is_some() {
        (StatusCode::UNAUTHORIZED, "invalid_token")
    } else {
        (StatusCode::UNAUTHORIZED, "missing_token")
    };
    tracing::info!(error, "bearer auth failed");
    (
        status,
        [(
            axum::http::header::WWW_AUTHENTICATE,
            challenge.as_str(),
        )],
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::{OAuthConfig, OAuthState};
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    fn test_oauth_state() -> OAuthState {
        let dir = std::env::temp_dir().join(format!(
            "zotero-mcp-http-test-{}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        OAuthState::with_tokens_path(
            OAuthConfig {
                client_id: "test-id".into(),
                client_secret: "test-secret".into(),
                issuer: "https://example.test".into(),
                access_token_ttl_secs: None,
                refresh_token_ttl_secs: None,
            },
            dir.join("tokens.json"),
        )
        .unwrap()
    }

    /// Build a router that mirrors the bearer-gated portion of `run`, without
    /// requiring `AppState`. The /sse route is replaced with a stub so we can
    /// observe whether the middleware lets the request through.
    fn protected_router(oauth_state: OAuthState) -> Router {
        Router::new()
            .route("/sse", get(|| async { StatusCode::OK }))
            .layer(axum::middleware::from_fn_with_state(
                oauth_state,
                require_bearer_token,
            ))
    }

    #[tokio::test]
    async fn missing_token_returns_401_with_www_authenticate() {
        let app = protected_router(test_oauth_state());
        let resp = app
            .oneshot(HttpRequest::builder().uri("/sse").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let challenge = resp
            .headers()
            .get(axum::http::header::WWW_AUTHENTICATE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(challenge.starts_with("Bearer "));
        assert!(challenge.contains("realm=\"zotero-mcp\""));
        assert!(challenge.contains(
            "resource_metadata=\"https://example.test/.well-known/oauth-protected-resource\""
        ));
        assert!(challenge.contains("scope=\"mcp\""));
    }

    #[tokio::test]
    async fn invalid_bearer_returns_401() {
        let app = protected_router(test_oauth_state());
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/sse")
                    .header("authorization", "Bearer not-a-real-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn minted_bearer_passes_through() {
        let oauth_state = test_oauth_state();
        let pair = oauth_state.token_store().mint_pair(None).await.unwrap();
        let token = pair.access_token;
        let app = protected_router(oauth_state);
        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/sse")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn tokens_survive_oauth_state_recreation() {
        let dir = tempfile::TempDir::new().unwrap();
        let tokens_path = dir.path().join("tokens.json");
        let config = OAuthConfig {
            client_id: "test-id".into(),
            client_secret: "test-secret".into(),
            issuer: "https://example.test".into(),
            access_token_ttl_secs: None,
            refresh_token_ttl_secs: None,
        };

        // Simulate the HTTP server's first lifetime: mint a token via the store
        // accessor, then drop everything as if launchd killed the process.
        let access_token = {
            let state_a = OAuthState::with_tokens_path(config.clone(), tokens_path.clone()).unwrap();
            let pair = state_a.token_store().mint_pair(None).await.unwrap();
            assert!(state_a.validate_token(&pair.access_token).await);
            pair.access_token
        };

        // Simulate launchd restart: brand-new OAuthState reading the same file.
        let state_b = OAuthState::with_tokens_path(config, tokens_path).unwrap();

        // The original access token MUST still validate. This is the regression
        // test for the in-memory-only token bug we shipped before.
        assert!(
            state_b.validate_token(&access_token).await,
            "access token issued before restart must still validate after restart"
        );
    }
}
