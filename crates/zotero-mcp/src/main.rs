mod http_transport;
mod logging;
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
        let token = std::env::var("ZOTERO_MCP_BEARER_TOKEN").map_err(|_| {
            anyhow::anyhow!(
                "ZOTERO_MCP_HTTP is set but ZOTERO_MCP_BEARER_TOKEN is missing. \
                 The HTTP transport requires a bearer token."
            )
        })?;
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
