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
        let addr: SocketAddr = bind
            .parse()
            .map_err(|e| anyhow::anyhow!("ZOTERO_MCP_HTTP must be host:port, got {bind:?}: {e}"))?;
        http_transport::run(state, addr, token).await
    } else {
        let server = server::ZoteroServer::new(state);
        let transport = (tokio::io::stdin(), tokio::io::stdout());
        let running = server.serve(transport).await?;
        running.waiting().await?;
        Ok(())
    }
}
