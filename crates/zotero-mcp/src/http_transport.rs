//! HTTP/streamable-HTTP transport for `zotero-mcp`.
//!
//! Mounts rmcp 1.7's `StreamableHttpService` at `/mcp`. The service handles
//! session management, request/response framing, and per-session
//! `ZoteroServer` spawning. We supply bearer auth as a tower layer on the
//! `/mcp` route only — OAuth discovery and `/oauth/token` stay
//! unauthenticated so clients can complete the handshake.
//!
//! Transport-config knobs are exposed as env vars (see spec Decision 2):
//!   - `ZOTERO_MCP_TRANSPORT_STATEFUL` (bool, default `true`)
//!   - `ZOTERO_MCP_TRANSPORT_JSON`     (bool, default `false`)
//!   - `ZOTERO_MCP_ALLOWED_HOSTS`      (comma-separated, default unset — uses
//!     rmcp's loopback-only default)

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager,
    tower::{StreamableHttpServerConfig, StreamableHttpService},
};
use tower_http::validate_request::ValidateRequestHeaderLayer;

use crate::oauth::{self, OAuthState};
use crate::server::ZoteroServer;
use crate::state::AppState;

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn env_allowed_hosts() -> Option<Vec<String>> {
    std::env::var("ZOTERO_MCP_ALLOWED_HOSTS").ok().map(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    })
}

pub async fn run(
    state: AppState,
    addr: SocketAddr,
    bearer: Option<String>,
    oauth_state: Option<OAuthState>,
) -> anyhow::Result<()> {
    let stateful_mode = env_bool("ZOTERO_MCP_TRANSPORT_STATEFUL", true);
    let json_response = env_bool("ZOTERO_MCP_TRANSPORT_JSON", false);
    let allowed_hosts_override = env_allowed_hosts();

    let mut config = StreamableHttpServerConfig::default()
        .with_sse_keep_alive(Some(Duration::from_secs(5)))
        .with_stateful_mode(stateful_mode)
        .with_json_response(json_response);
    if let Some(hosts) = allowed_hosts_override {
        config = config.with_allowed_hosts(hosts);
    }

    let state_for_factory = state.clone();
    let service = StreamableHttpService::new(
        move || Ok(ZoteroServer::new(state_for_factory.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let mut mcp_route = Router::new().route_service("/mcp", service);
    if let Some(oauth_state) = oauth_state.clone() {
        mcp_route = mcp_route.layer(axum::middleware::from_fn_with_state(
            oauth_state,
            crate::bearer::require_bearer_token,
        ));
    }

    let mut app = Router::new().merge(mcp_route);
    if let Some(oauth_state) = oauth_state {
        app = app.merge(oauth::router(oauth_state));
        tracing::info!(
            "OAuth 2.1 surface mounted (discovery + /oauth/token + bearer gate on /mcp)"
        );
    }
    let mut app = app.layer(tower_http::trace::TraceLayer::new_for_http());

    if let Some(token) = bearer {
        #[allow(deprecated)]
        let auth = ValidateRequestHeaderLayer::bearer(&token);
        app = app.layer(auth);
        tracing::info!(
            %addr,
            stateful_mode,
            json_response,
            "zotero-mcp streamable-HTTP transport listening (static bearer auth enabled)"
        );
    } else {
        tracing::warn!(
            %addr,
            stateful_mode,
            json_response,
            "zotero-mcp streamable-HTTP transport listening WITHOUT static bearer — \
             OAuth gates /mcp if configured; otherwise transport-level access control applies"
        );
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::{OAuthConfig, OAuthState};
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
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

    #[tokio::test]
    async fn mcp_route_rejects_request_without_bearer() {
        use axum::routing::get;
        let oauth_state = test_oauth_state();
        let stub = Router::new()
            .route("/mcp", get(|| async { StatusCode::OK }))
            .layer(axum::middleware::from_fn_with_state(
                oauth_state,
                crate::bearer::require_bearer_token,
            ));
        let resp = stub
            .oneshot(
                HttpRequest::builder()
                    .uri("/mcp")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(
            resp.headers()
                .get(axum::http::header::WWW_AUTHENTICATE)
                .is_some()
        );
    }
}
