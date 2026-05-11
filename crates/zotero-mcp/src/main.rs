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
        // OAuth 2.1 client credentials. Load from <config_dir>/oauth.toml; if
        // missing, generate iff ZOTERO_MCP_OAUTH_ISSUER is set (one-time
        // bootstrap that writes the file with mode 0600). Otherwise, OAuth is
        // disabled — the server runs without an auth gate, as before.
        let issuer_hint =
            std::env::var("ZOTERO_MCP_OAUTH_ISSUER").ok().filter(|s| !s.is_empty());
        let oauth_state = oauth::OAuthConfig::load_or_generate(issuer_hint)?
            .map(oauth::OAuthState::new);
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
