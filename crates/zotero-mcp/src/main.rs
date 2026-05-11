mod http_transport;
mod logging;
mod oauth;
mod resources;
mod server;
mod state;
mod tools;

use rmcp::ServiceExt;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = zotero_core::Config::load().unwrap_or_default();
    logging::init(&cfg.logging.level, Some(&cfg.resolved_log_dir()))?;
    tracing::info!(
        "zotero-mcp starting (user_id auto-detect = {})",
        cfg.zotero.user_id == 0
    );
    let state = state::AppState::build(cfg).await?;

    if let Ok(bind) = std::env::var("ZOTERO_MCP_HTTP") {
        // Bearer token is now optional. Some MCP clients (notably Claude.ai's
        // "Add custom connector" flow) probe discovery endpoints without
        // sending an Authorization header and expect a useful response; a
        // blanket bearer check breaks that handshake. When the env var is
        // empty or unset, the HTTP server runs without auth — security then
        // depends on the transport (e.g., a private Tailscale Funnel URL).
        let token = std::env::var("ZOTERO_MCP_BEARER_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        // Task B/D: OAuth 2.1 surface. Stopgap env-var config until Task D
        // moves it onto disk. All three must be present to enable OAuth.
        let oauth_state = match (
            std::env::var("ZOTERO_MCP_OAUTH_CLIENT_ID").ok(),
            std::env::var("ZOTERO_MCP_OAUTH_CLIENT_SECRET").ok(),
            std::env::var("ZOTERO_MCP_OAUTH_ISSUER").ok(),
        ) {
            (Some(client_id), Some(client_secret), Some(issuer))
                if !client_id.is_empty()
                    && !client_secret.is_empty()
                    && !issuer.is_empty() =>
            {
                Some(oauth::OAuthState::new(oauth::OAuthConfig {
                    client_id,
                    client_secret,
                    issuer,
                }))
            }
            _ => None,
        };
        let addr: SocketAddr = bind
            .parse()
            .map_err(|e| anyhow::anyhow!("ZOTERO_MCP_HTTP must be host:port, got {bind:?}: {e}"))?;
        http_transport::run(state, addr, token, oauth_state).await
    } else {
        let server = server::ZoteroServer::new(state);
        let transport = (tokio::io::stdin(), tokio::io::stdout());
        let running = server.serve(transport).await?;
        running.waiting().await?;
        Ok(())
    }
}
