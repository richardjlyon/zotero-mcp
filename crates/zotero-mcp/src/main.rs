use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use std::net::SocketAddr;
use zotero_mcp::{core as zcore, http_transport, logging, oauth, server, setup, state};

#[derive(Parser)]
#[command(
    name = "zotero-mcp",
    about = "Local-first Zotero bridge: runs as an MCP server over stdio (default), \
             HTTP/SSE (when ZOTERO_MCP_HTTP is set), or a setup helper for the HTTP \
             deployment."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive setup: detect Tailscale, write launchd plist, enable
    /// Funnel, generate OAuth credentials, and print the values to paste into
    /// the Claude.ai connector. macOS-only.
    Setup,
    /// Read-only health check covering launchd, the HTTP server, Tailscale
    /// Funnel, the Zotero local API, and the OAuth config file.
    Status,
    /// Print the current OAuth client_id, client_secret, and connector URL
    /// from <config_dir>/oauth.toml in paste-ready form.
    ShowCredentials,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Setup) => setup::run_setup().await,
        Some(Command::Status) => setup::run_status().await,
        Some(Command::ShowCredentials) => setup::run_show_credentials(),
        None => run_server().await,
    }
}

async fn run_server() -> anyhow::Result<()> {
    let cfg = zcore::Config::load().unwrap_or_default();
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
        let oauth_state = match oauth::OAuthConfig::load_or_generate(issuer_hint)? {
            Some(cfg) => Some(oauth::OAuthState::from_default_path(cfg)?),
            None => None,
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
