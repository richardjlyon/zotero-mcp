use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use std::net::SocketAddr;
use zotero_mcp::{core as zcore, http_transport, logging, oauth, server, setup, state};

#[derive(Parser)]
#[command(
    name = "zotero-mcp",
    about = "Local-first Zotero bridge: runs as an MCP server over stdio (default), \
             HTTP (when ZOTERO_MCP_HTTP is set), or a setup helper for the HTTP \
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
    /// Extract a PDF attachment's text to stdout — the same layout-aware,
    /// OCR-capable engine as the `get_pdf_text` MCP tool. Whole-document by
    /// default; a large scan that exceeds the whole-document page limit is
    /// walked in page windows internally and streamed to stdout in full, so
    /// callers (e.g. a fact-check arbiter) get complete text in one command
    /// without orchestrating the window loop themselves. A diagnostic header
    /// (route, pages, completeness) is written to stderr; stdout is only the
    /// document text.
    PdfText {
        /// The parent item key (not the attachment key).
        item_key: String,
        /// First page of an explicit window (1-indexed, inclusive).
        #[arg(long)]
        from: Option<u32>,
        /// Last page of an explicit window (1-indexed, inclusive).
        #[arg(long)]
        to: Option<u32>,
        /// Force the flat-text path (no Docling, no OCR, no page anchors).
        #[arg(long)]
        plain: bool,
        /// Page-window size used when auto-walking a large document.
        #[arg(long, default_value_t = 25)]
        window_size: u32,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Setup) => setup::run_setup().await,
        Some(Command::Status) => setup::run_status().await,
        Some(Command::ShowCredentials) => setup::run_show_credentials(),
        Some(Command::PdfText {
            item_key,
            from,
            to,
            plain,
            window_size,
        }) => run_pdf_text(item_key, from, to, plain, window_size).await,
        None => run_server().await,
    }
}

/// `pdf-text` subcommand: extract an item's PDF via the same engine stack as
/// the MCP tool, printing the document text to stdout. Large scans are walked
/// in windows internally (stdout has no response-size ceiling, unlike an MCP
/// tool result), so one command yields the whole document.
async fn run_pdf_text(
    item_key: String,
    from: Option<u32>,
    to: Option<u32>,
    plain: bool,
    window_size: u32,
) -> anyhow::Result<()> {
    use std::io::Write;
    use zcore::pdf::{get_pdf_text_stored, ServedFrom};

    let cfg = zcore::Config::load().unwrap_or_default();
    // Logging goes to the log dir (never stdout), so stdout stays pure text.
    logging::init(&cfg.logging.level, Some(&cfg.resolved_log_dir()))?;
    let state = state::AppState::build(cfg).await?;
    let storage = state.cfg.storage_dir();
    let _ = window_size; // window walking is now internal to the store builder

    let header = |r: &zcore::pdf::PdfTextResult, span: &str| {
        eprintln!(
            "[pdf-text {item_key} | route: {:?} | served: {} | pages: {} | total: {} | complete: {} | {span}]",
            r.source,
            match r.served_from {
                ServedFrom::Store => "store",
                ServedFrom::Fresh => "fresh",
            },
            r.completeness.pages,
            r.completeness.total_pages,
            r.completeness.complete
        );
    };

    // Explicit window, or the whole document. Either way the store is
    // consulted first and the walk over a large document happens inside
    // `get_pdf_text_stored`, so this command no longer orchestrates windows
    // itself — and a second run of it costs nothing.
    let window = match (from, to) {
        (None, None) => None,
        (f, t) => {
            let f = f.unwrap_or(1).max(1);
            Some((f, t.unwrap_or(f).max(f)))
        }
    };
    let r = get_pdf_text_stored(
        &state.pool,
        &item_key,
        1,
        &storage,
        &state.pdf_engines,
        &state.derivatives,
        plain,
        window,
        false,
    )
    .await?;
    header(
        &r,
        &match window {
            Some((f, t)) => format!("window {f}..={t}"),
            None => "whole document".to_string(),
        },
    );
    print!("{}", r.text);
    std::io::stdout().flush().ok();
    Ok(())
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
        let issuer_hint = std::env::var("ZOTERO_MCP_OAUTH_ISSUER")
            .ok()
            .filter(|s| !s.is_empty());
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
