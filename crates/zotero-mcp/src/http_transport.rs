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
    extract::{Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
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

pub async fn run(state: AppState, addr: SocketAddr, bearer: String) -> anyhow::Result<()> {
    let shared = AppShared {
        state,
        txs: Default::default(),
    };
    // tower-http marks the simple `bearer` constructor as deprecated in favour
    // of writing a custom validator, but a constant-time shared-secret check is
    // exactly what we want here.
    #[allow(deprecated)]
    let auth = ValidateRequestHeaderLayer::bearer(&bearer);
    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/message", post(post_handler))
        .with_state(shared)
        .layer(auth);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "zotero-mcp HTTP/SSE transport listening");
    axum::serve(listener, app).await?;
    Ok(())
}
